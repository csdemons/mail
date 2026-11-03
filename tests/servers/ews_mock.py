"""Имитация Exchange Web Services (EWS) для тестов: ящик с папками и письмами,
вход NTLMv2 (проверка ответа по паролю, как у сервера) и Autodiscover.
Форматы — по документации Microsoft (EWS SOAP, MS-NLMP).
python3 tests/servers/ews_mock.py <cert> <key> [port=8445]

Учётная запись: CORP\\ivanov (или ivanov@exch.test), пароль Exch-Pass-1, адрес ivanov@exch.test.

Тестовые ручки:
  POST /test/deliver            — тело: письмо (MIME) во «Входящие»
  GET  /test/state              — папки и письма (без исходников)
  POST /test/remove?subject=... — удалить письмо «в веб-почте»
"""
import base64, email, email.policy, email.utils, hashlib, hmac, itertools, json, re, ssl, struct, sys, threading, time
import xml.etree.ElementTree as ET
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

USER, DOMAIN, PASSWORD, ADDRESS = 'ivanov', 'CORP', 'Exch-Pass-1', 'ivanov@exch.test'
NS = {'soap': 'http://schemas.xmlsoap.org/soap/envelope/', 't': 'http://schemas.microsoft.com/exchange/services/2006/types', 'm': 'http://schemas.microsoft.com/exchange/services/2006/messages'}
lock = threading.Lock()
seq = itertools.count(1)


# ---------------------------------------------------------------- MD4 (hashlib без OpenSSL legacy его не умеет)
def md4(data):
    def f(x, y, z): return (x & y) | (~x & z)
    def g(x, y, z): return (x & y) | (x & z) | (y & z)
    def h(x, y, z): return x ^ y ^ z
    def rol(v, s): v &= 0xffffffff; return ((v << s) | (v >> (32 - s))) & 0xffffffff
    msg = bytearray(data); ml = len(data) * 8
    msg.append(0x80)
    while len(msg) % 64 != 56: msg.append(0)
    msg += struct.pack('<Q', ml)
    a, b, c, d = 0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476
    for i in range(0, len(msg), 64):
        X = struct.unpack('<16I', msg[i:i + 64]); aa, bb, cc, dd = a, b, c, d
        for k in range(16):
            s = [3, 7, 11, 19][k % 4]
            if k % 4 == 0: a = rol(a + f(b, c, d) + X[k], s)
            elif k % 4 == 1: d = rol(d + f(a, b, c) + X[k], s)
            elif k % 4 == 2: c = rol(c + f(d, a, b) + X[k], s)
            else: b = rol(b + f(c, d, a) + X[k], s)
        for k in range(16):
            idx = [0, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15][k]; s = [3, 5, 9, 13][k % 4]
            if k % 4 == 0: a = rol(a + g(b, c, d) + X[idx] + 0x5a827999, s)
            elif k % 4 == 1: d = rol(d + g(a, b, c) + X[idx] + 0x5a827999, s)
            elif k % 4 == 2: c = rol(c + g(d, a, b) + X[idx] + 0x5a827999, s)
            else: b = rol(b + g(c, d, a) + X[idx] + 0x5a827999, s)
        for k in range(16):
            idx = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15][k]; s = [3, 9, 11, 15][k % 4]
            if k % 4 == 0: a = rol(a + h(b, c, d) + X[idx] + 0x6ed9eba1, s)
            elif k % 4 == 1: d = rol(d + h(a, b, c) + X[idx] + 0x6ed9eba1, s)
            elif k % 4 == 2: c = rol(c + h(d, a, b) + X[idx] + 0x6ed9eba1, s)
            else: b = rol(b + h(c, d, a) + X[idx] + 0x6ed9eba1, s)
        a, b, c, d = (a + aa) & 0xffffffff, (b + bb) & 0xffffffff, (c + cc) & 0xffffffff, (d + dd) & 0xffffffff
    return struct.pack('<4I', a, b, c, d)

assert md4(b'abc').hex() == 'a448017aaf21d8525fc10ae87aa6729d'


def ntowfv2(user, domain, password):
    nt = md4(password.encode('utf-16-le'))
    return hmac.new(nt, (user.upper() + domain).encode('utf-16-le'), 'md5').digest()


def challenge_msg(server_challenge):
    info = b''
    for t, v in ((2, DOMAIN), (1, 'EXCH01'), (7, None)):
        val = struct.pack('<Q', 133_000_000_000_000_000) if v is None else v.encode('utf-16-le')
        info += struct.pack('<HH', t, len(val)) + val
    info += b'\0\0\0\0'
    flags = 0x00000001 | 0x00000200 | 0x00080000 | 0x00800000 | 0x02000000 | 0x20000000 | 0x00008000 | 0x4
    name = DOMAIN.encode('utf-16-le')
    head = b'NTLMSSP\0' + struct.pack('<I', 2) + struct.pack('<HHI', len(name), len(name), 56) + struct.pack('<I', flags) + server_challenge + b'\0' * 8
    head += struct.pack('<HHI', len(info), len(info), 56 + len(name)) + bytes([10, 0, 0x61, 0x4a, 0, 0, 0, 15])
    return head + name + info


def field(msg, at):
    ln, _, off = struct.unpack('<HHI', msg[at:at + 8])
    return msg[off:off + ln]


def verify_auth(msg, server_challenge):
    """NTLMv2: проверка NTProofStr по паролю учётной записи и наличия привязки к каналу."""
    if msg[:8] != b'NTLMSSP\0' or struct.unpack('<I', msg[8:12])[0] != 3:
        return False, 'not type 3'
    nt = field(msg, 20); dom = field(msg, 28).decode('utf-16-le'); user = field(msg, 36).decode('utf-16-le')
    ok_user = (user.lower() == USER and dom.upper() == DOMAIN) or (user.lower() == ADDRESS and dom == '')
    if not ok_user or len(nt) < 48:
        return False, f'user {dom}\\{user}'
    key = ntowfv2(user, dom, PASSWORD)
    proof = hmac.new(key, server_challenge + nt[16:], 'md5').digest()
    if proof != nt[:16]:
        return False, 'bad password'
    # AV-пары в blob: MsvAvChannelBindings (10) должна быть.
    info, i, has_cb = nt[16 + 28:], 0, False
    while i + 4 <= len(info):
        t, ln = struct.unpack('<HH', info[i:i + 4])
        if t == 0: break
        if t == 10 and info[i + 4:i + 4 + ln] != b'\0' * 16: has_cb = True
        i += 4 + ln
    return has_cb, 'no channel binding' if not has_cb else 'ok'


# ---------------------------------------------------------------- ящик
def now():
    return time.time()

def iso(ts):
    return time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime(ts))

def parse_iso(s):
    return time.mktime(time.strptime(s.replace('Z', ''), '%Y-%m-%dT%H:%M:%S')) - time.timezone

FOLDERS = {
    'F-root': {'name': 'Top of Information Store', 'parent': None, 'class': 'IPF.Note', 'dist': 'msgfolderroot'},
    'F-inbox': {'name': 'Входящие', 'parent': 'F-root', 'class': 'IPF.Note', 'dist': 'inbox'},
    'F-drafts': {'name': 'Черновики', 'parent': 'F-root', 'class': 'IPF.Note', 'dist': 'drafts'},
    'F-sent': {'name': 'Отправленные', 'parent': 'F-root', 'class': 'IPF.Note', 'dist': 'sentitems'},
    'F-deleted': {'name': 'Удаленные', 'parent': 'F-root', 'class': 'IPF.Note', 'dist': 'deleteditems'},
    'F-junk': {'name': 'Нежелательная почта', 'parent': 'F-root', 'class': 'IPF.Note', 'dist': 'junkemail'},
    'F-proj': {'name': 'Проекты', 'parent': 'F-inbox', 'class': 'IPF.Note', 'dist': None},
    'F-cal': {'name': 'Календарь', 'parent': 'F-root', 'class': 'IPF.Appointment', 'dist': 'calendar'},
    'F-hidden': {'name': 'Sync Issues', 'parent': 'F-root', 'class': 'IPF.Note', 'dist': None, 'hidden': True},
}
ITEMS = {}   # id -> {folder, mime, received, read, flag, importance, categories, verb}
SENT = []    # отправленные через SendAndSaveCopy (MIME)


def mime_of(frm, to, subject, text, attachments=(), msgid=None, date=None):
    m = email.message.EmailMessage()
    m['From'] = frm; m['To'] = to; m['Subject'] = subject
    m['Date'] = email.utils.formatdate(date or now(), localtime=True)
    m['Message-ID'] = msgid or f'<ews-{next(seq)}@exch.test>'
    m.set_content(text)
    for name, data in attachments:
        m.add_attachment(data, maintype='application', subtype='octet-stream', filename=name)
    return m.as_bytes(policy=email.policy.SMTP)


def add_item(folder, mime, received=None, read=False, **kw):
    iid = f'AAMk-{next(seq)}-' + base64.b64encode(f'{folder}/{time.time()}'.encode()).decode()
    ITEMS[iid] = {'folder': folder, 'mime': mime, 'received': received or now(), 'read': read, 'flag': False, 'importance': 'Normal', 'categories': [], 'verb': None, **kw}
    return iid


T0 = now()
add_item('F-inbox', mime_of('Петров Пётр <petrov@exch.test>', ADDRESS, 'EWS: квартальный отчёт', 'Отчёт во вложении', [('Отчёт Q3.xlsx', b'XLSX-Q3')]), T0 - 3600)
add_item('F-inbox', mime_of('Сидорова <sidorova@partner.ru>', ADDRESS, 'EWS: приглашение на семинар', 'Приглашаем на семинар по безопасности'), T0 - 7200, read=True)
add_item('F-inbox', mime_of('noreply@exch.test', ADDRESS, 'EWS: старое письмо', 'Совсем старое', date=T0 - 86400 * 40), T0 - 86400 * 40, read=True)
add_item('F-proj', mime_of('Петров Пётр <petrov@exch.test>', ADDRESS, 'EWS: план проекта', 'План работ на ноябрь'), T0 - 600)
add_item('F-sent', mime_of(ADDRESS, 'petrov@exch.test', 'EWS: ответ по отчёту', 'Спасибо'), T0 - 300, read=True)


def parsed(item):
    return email.message_from_bytes(item['mime'], policy=email.policy.default)


def addr_xml(tag, value):
    out = []
    for name, addr in email.utils.getaddresses([value or '']):
        if addr or name:
            out.append(f'<t:Mailbox><t:Name>{esc(name or addr)}</t:Name><t:EmailAddress>{esc(addr)}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox>')
    return f'<t:{tag}>{"".join(out)}</t:{tag}>' if out else ''


def esc(s):
    return (s or '').replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;').replace('"', '&quot;')


def item_xml(iid, props, mime=False):
    it = ITEMS[iid]; p = parsed(it)
    x = f'<t:ItemId Id="{iid}" ChangeKey="CK{int(it["received"])}"/>'
    if mime:
        x = f'<t:MimeContent CharacterSet="UTF-8">{base64.b64encode(it["mime"]).decode()}</t:MimeContent>' + x
    want = lambda f: f in props
    if want('item:Subject'): x += f'<t:Subject>{esc(str(p["subject"] or ""))}</t:Subject>'
    if want('item:DateTimeReceived'): x += f'<t:DateTimeReceived>{iso(it["received"])}</t:DateTimeReceived>'
    if want('item:Size'): x += f'<t:Size>{len(it["mime"])}</t:Size>'
    if want('item:Categories') and it['categories']: x += '<t:Categories>' + ''.join(f'<t:String>{esc(c)}</t:String>' for c in it['categories']) + '</t:Categories>'
    if want('item:Importance'): x += f'<t:Importance>{it["importance"]}</t:Importance>'
    if want('item:InReplyTo') and p['in-reply-to']: x += f'<t:InReplyTo>{esc(str(p["in-reply-to"]))}</t:InReplyTo>'
    atts = [part for part in p.iter_attachments()]
    if want('item:HasAttachments'): x += f'<t:HasAttachments>{"true" if atts else "false"}</t:HasAttachments>'
    if want('item:Attachments') and atts:
        x += '<t:Attachments>' + ''.join(f'<t:FileAttachment><t:AttachmentId Id="A{i}"/><t:Name>{esc(a.get_filename() or "file")}</t:Name><t:IsInline>false</t:IsInline></t:FileAttachment>' for i, a in enumerate(atts)) + '</t:Attachments>'
    if want('0x1081') and it['verb']: x += f'<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x1081" PropertyType="Integer"/><t:Value>{it["verb"]}</t:Value></t:ExtendedProperty>'
    if want('item:Flag'): x += f'<t:Flag><t:FlagStatus>{"Flagged" if it["flag"] else "NotFlagged"}</t:FlagStatus></t:Flag>'
    if want('message:From'): x += addr_xml('From', str(p['from'] or '')).replace('<t:From>', '<t:From>', 1)
    if want('message:ToRecipients'): x += addr_xml('ToRecipients', str(p['to'] or ''))
    if want('message:IsRead'): x += f'<t:IsRead>{"true" if it["read"] else "false"}</t:IsRead>'
    if want('message:InternetMessageId') and p['message-id']: x += f'<t:InternetMessageId>{esc(str(p["message-id"]))}</t:InternetMessageId>'
    if want('message:References') and p['references']: x += f'<t:References>{esc(str(p["references"]))}</t:References>'
    return f'<t:Message>{x}</t:Message>'


def props_of(shape):
    out = set()
    for f in shape.iter():
        if f.get('FieldURI'): out.add(f.get('FieldURI'))
        if f.get('PropertyTag'): out.add(f.get('PropertyTag').lower())
    return out


def folder_ref(el):
    """FolderId или DistinguishedFolderId → наш id папки."""
    if el is None: return None
    for c in el.iter():
        tag = c.tag.split('}')[1]
        if tag == 'FolderId': return c.get('Id') if c.get('Id') in FOLDERS else None
        if tag == 'DistinguishedFolderId':
            return next((k for k, v in FOLDERS.items() if v['dist'] == c.get('Id')), None)
    return None


def resp(op, body, cls='Success', code='NoError', text=None):
    msg = f'<m:MessageText>{esc(text)}</m:MessageText>' if text else ''
    return f'<m:{op}ResponseMessage ResponseClass="{cls}">{msg}<m:ResponseCode>{code}</m:ResponseCode>{body}</m:{op}ResponseMessage>'


def envelope(op, messages):
    return ('<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Header><h:ServerVersionInfo xmlns:h="http://schemas.microsoft.com/exchange/services/2006/types" MajorVersion="15" MinorVersion="2"/></s:Header><s:Body>'
            f'<m:{op}Response xmlns:m="{NS["m"]}" xmlns:t="{NS["t"]}"><m:ResponseMessages>{"".join(messages)}</m:ResponseMessages></m:{op}Response></s:Body></s:Envelope>')


def counts(fid):
    items = [i for i in ITEMS.values() if i['folder'] == fid]
    return len(items), sum(1 for i in items if not i['read'])


def folder_xml(fid, props=True):
    f = FOLDERS[fid]; total, unread = counts(fid)
    x = f'<t:FolderId Id="{fid}" ChangeKey="1"/>'
    if props:
        if f['parent']: x += f'<t:ParentFolderId Id="{f["parent"]}"/>'
        x += f'<t:FolderClass>{f["class"]}</t:FolderClass><t:DisplayName>{esc(f["name"])}</t:DisplayName><t:TotalCount>{total}</t:TotalCount><t:ChildFolderCount>{sum(1 for v in FOLDERS.values() if v["parent"] == fid)}</t:ChildFolderCount><t:UnreadCount>{unread}</t:UnreadCount>'
        x += f'<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag="0x10f4" PropertyType="Boolean"/><t:Value>{"true" if f.get("hidden") else "false"}</t:Value></t:ExtendedProperty>'
    return f'<t:Folder>{x}</t:Folder>'


def restriction_ok(el, it):
    if el is None: return True
    tag = el.tag.split('}')[1]
    kids = list(el)
    if tag in ('Restriction', 'And'): return all(restriction_ok(k, it) for k in kids)
    if tag == 'Or': return any(restriction_ok(k, it) for k in kids)
    field = kids[0].get('FieldURI'); const = el.find('.//t:Constant', NS).get('Value')
    if field == 'item:DateTimeReceived':
        v, c = it['received'], parse_iso(const)
        return {'IsLessThanOrEqualTo': v <= c + 0.999, 'IsGreaterThanOrEqualTo': v >= c, 'IsLessThan': v < c, 'IsEqualTo': int(v) == int(c)}[tag]
    if field == 'message:IsRead': return it['read'] == (const == 'true')
    if field == 'message:InternetMessageId': return str(parsed(it)['message-id']).strip().lower() == const.strip().lower()
    return True


def aqs_ok(q, it):
    p = parsed(it); body = p.get_body(('plain',))
    text = (str(p['subject']) + ' ' + str(p['from']) + ' ' + (body.get_content() if body else '')).lower()
    for part in re.findall(r'(\w+:)?"([^"]*)"|(\S+)', q):
        pre, quoted, bare = part
        word = (quoted or bare).lower()
        if bare in ('AND', 'OR'): continue
        if pre == 'subject:' and word not in str(p['subject']).lower(): return False
        if pre == 'from:' and word not in str(p['from']).lower(): return False
        if not pre and word not in text: return False
    return True


def handle(op, el):
    """Операция EWS → (имя ответа, список ResponseMessage)."""
    if op == 'GetFolder':
        out = []
        for ref in el.find('m:FolderIds', NS):
            tag = ref.tag.split('}')[1]
            fid = (next((k for k, v in FOLDERS.items() if v['dist'] == ref.get('Id')), None) if tag == 'DistinguishedFolderId' else (ref.get('Id') if ref.get('Id') in FOLDERS else None))
            out.append(resp(op, f'<m:Folders>{folder_xml(fid)}</m:Folders>') if fid else resp(op, '', 'Error', 'ErrorFolderNotFound', 'The specified folder could not be found in the store.'))
        return out
    if op == 'FindFolder':
        view = el.find('m:IndexedPageFolderView', NS); off = int(view.get('Offset', 0)); mx = int(view.get('MaxEntriesReturned', 1000))
        allf = [k for k in FOLDERS if k != 'F-root']
        page = allf[off:off + mx]
        return [resp(op, f'<m:RootFolder IndexedPagingOffset="{off + len(page)}" TotalItemsInView="{len(allf)}" IncludesLastItemInRange="{"true" if off + len(page) >= len(allf) else "false"}"><t:Folders>{"".join(folder_xml(f) for f in page)}</t:Folders></m:RootFolder>')]
    if op == 'FindItem':
        fid = folder_ref(el.find('m:ParentFolderIds', NS))
        if not fid: return [resp(op, '', 'Error', 'ErrorFolderNotFound', 'no folder')]
        items = [(k, v) for k, v in ITEMS.items() if v['folder'] == fid and restriction_ok(el.find('m:Restriction', NS), v)]
        q = el.find('m:QueryString', NS)
        if q is not None: items = [(k, v) for k, v in items if aqs_ok(q.text or '', v)]
        items.sort(key=lambda kv: -kv[1]['received'])
        view = el.find('m:IndexedPageItemView', NS); off = int(view.get('Offset', 0)); mx = int(view.get('MaxEntriesReturned', 1000))
        page = items[off:off + mx]
        props = props_of(el.find('m:ItemShape', NS))
        return [resp(op, f'<m:RootFolder IndexedPagingOffset="{off + len(page)}" TotalItemsInView="{len(items)}" IncludesLastItemInRange="{"true" if off + len(page) >= len(items) else "false"}"><t:Items>{"".join(item_xml(k, props) for k, _ in page)}</t:Items></m:RootFolder>')]
    if op == 'GetItem':
        shape = el.find('m:ItemShape', NS); props = props_of(shape)
        mime = (shape.findtext('t:IncludeMimeContent', '', NS) == 'true')
        out = []
        for i in el.find('m:ItemIds', NS):
            iid = i.get('Id')
            out.append(resp(op, f'<m:Items>{item_xml(iid, props, mime)}</m:Items>') if iid in ITEMS else resp(op, '', 'Error', 'ErrorItemNotFound', 'The specified object was not found in the store.'))
        return out
    if op == 'UpdateItem':
        out = []
        for ch in el.find('m:ItemChanges', NS):
            iid = ch.find('t:ItemId', NS).get('Id')
            if iid not in ITEMS: out.append(resp(op, '', 'Error', 'ErrorItemNotFound', 'not found')); continue
            it = ITEMS[iid]
            for upd in ch.find('t:Updates', NS):
                f = upd.find('t:FieldURI', NS).get('FieldURI'); val = upd.find('t:Message', NS)
                if upd.tag.endswith('DeleteItemField'):
                    if f == 'item:Categories': it['categories'] = []
                    continue
                if f == 'message:IsRead': it['read'] = val.findtext('t:IsRead', '', NS) == 'true'
                if f == 'item:Flag': it['flag'] = val.findtext('t:Flag/t:FlagStatus', '', NS) == 'Flagged'
                if f == 'item:Importance': it['importance'] = val.findtext('t:Importance', '', NS)
                if f == 'item:Categories': it['categories'] = [s.text for s in val.findall('t:Categories/t:String', NS)]
            out.append(resp(op, f'<m:Items><t:Message><t:ItemId Id="{iid}" ChangeKey="CK2"/></t:Message></m:Items><m:ConflictResults><t:Count>0</t:Count></m:ConflictResults>'))
        return out
    if op == 'MoveItem':
        to = folder_ref(el.find('m:ToFolderId', NS)); out = []
        for i in el.find('m:ItemIds', NS):
            iid = i.get('Id')
            if iid not in ITEMS: out.append(resp(op, '', 'Error', 'ErrorItemNotFound', 'not found')); continue
            it = ITEMS.pop(iid); new = add_item(to, it['mime'], it['received'], it['read'], flag=it['flag'], importance=it['importance'], categories=it['categories'])
            out.append(resp(op, f'<m:Items><t:Message><t:ItemId Id="{new}" ChangeKey="CK1"/></t:Message></m:Items>'))
        return out
    if op == 'DeleteItem':
        out = []
        for i in el.find('m:ItemIds', NS):
            iid = i.get('Id')
            if iid not in ITEMS: out.append(resp(op, '', 'Error', 'ErrorItemNotFound', 'not found')); continue
            if el.get('DeleteType') == 'MoveToDeletedItems':
                it = ITEMS.pop(iid); add_item('F-deleted', it['mime'], it['received'], it['read'])
            else:
                ITEMS.pop(iid)
            out.append(resp(op, ''))
        return out
    if op == 'CreateItem':
        disp = el.get('MessageDisposition'); target = folder_ref(el.find('m:SavedItemFolderId', NS)) or 'F-drafts'
        out = []
        for msg in el.find('m:Items', NS):
            raw = base64.b64decode(msg.findtext('t:MimeContent', '', NS))
            read = msg.find('.//t:ExtendedProperty', NS) is not None
            if disp == 'SendAndSaveCopy':
                SENT.append(raw); add_item('F-sent', raw, now(), True)
            else:
                add_item(target, raw, now(), read)
            out.append(resp(op, '<m:Items/>'))
        return out
    if op == 'CreateFolder':
        parent = folder_ref(el.find('m:ParentFolderId', NS)); name = el.findtext('.//t:DisplayName', '', NS)
        if any(v['parent'] == parent and v['name'] == name for v in FOLDERS.values()):
            return [resp(op, '', 'Error', 'ErrorFolderExists', 'exists')]
        fid = f'F-{next(seq)}'; FOLDERS[fid] = {'name': name, 'parent': parent, 'class': 'IPF.Note', 'dist': None}
        return [resp(op, f'<m:Folders><t:Folder><t:FolderId Id="{fid}"/></t:Folder></m:Folders>')]
    return [resp(op, '', 'Error', 'ErrorInvalidRequest', f'unsupported {op}')]


AUTODISCOVER = '''<?xml version="1.0" encoding="utf-8"?><Autodiscover xmlns="http://schemas.microsoft.com/exchange/autodiscover/responseschema/2006"><Response xmlns="http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a"><User><DisplayName>Иванов</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action><Protocol><Type>EXCH</Type><EwsUrl>https://localhost:{port}/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>'''


class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *a):
        pass

    def send(self, code, body=b'', ctype='text/xml; charset=utf-8', headers=()):
        if isinstance(body, str): body = body.encode()
        self.send_response(code)
        for k, v in headers: self.send_header(k, v)
        self.send_header('Content-Type', ctype); self.send_header('Content-Length', str(len(body)))
        self.end_headers(); self.wfile.write(body)

    def authed(self):
        """NTLM на соединении: тип 1 → вызов; тип 3 — проверка; дальше соединение помнит вход."""
        if getattr(self, '_ok', False) or getattr(self.connection, 'ntlm_ok', False):
            return True
        a = self.headers.get('Authorization', '')
        scheme, _, tok = a.partition(' ')
        if scheme not in ('NTLM', 'Negotiate') or not tok:
            self.send(401, 'auth', headers=[('WWW-Authenticate', 'Negotiate'), ('WWW-Authenticate', 'NTLM')]); return False
        msg = base64.b64decode(tok)
        kind = struct.unpack('<I', msg[8:12])[0]
        if kind == 1:
            ch = bytes(range(8, 16)); self.connection.ntlm_challenge = ch
            self.send(401, '', headers=[('WWW-Authenticate', f'{scheme} ' + base64.b64encode(challenge_msg(ch)).decode())]); return False
        ok, why = verify_auth(msg, getattr(self.connection, 'ntlm_challenge', b''))
        with lock: STATS['auth'].append(why)
        if not ok:
            self.send(401, why, headers=[('WWW-Authenticate', 'NTLM')]); return False
        self.connection.ntlm_ok = True
        return True

    def do_POST(self):
        u = urlparse(self.path); n = int(self.headers.get('Content-Length', 0)); body = self.rfile.read(n)
        if u.path == '/test/deliver':
            with lock: add_item('F-inbox', body, now())
            return self.send(200, 'ok', 'text/plain')
        if u.path == '/test/remove':
            subj = parse_qs(u.query)['subject'][0]
            with lock:
                for k in [k for k, v in ITEMS.items() if str(parsed(v)['subject']) == subj]: ITEMS.pop(k)
            return self.send(200, 'ok', 'text/plain')
        if not self.authed():
            return
        if u.path.lower() == '/autodiscover/autodiscover.xml':
            return self.send(200, AUTODISCOVER.format(port=self.server.server_address[1]))
        if u.path.lower() != '/ews/exchange.asmx':
            return self.send(404, 'not found', 'text/plain')
        root = ET.fromstring(body)
        op_el = root.find('soap:Body', NS)[0]
        op = op_el.tag.split('}')[1]
        with lock:
            STATS['ops'].append(op)
            msgs = handle(op, op_el)
        self.send(200, envelope(op, msgs))

    def do_GET(self):
        if urlparse(self.path).path == '/test/state':
            with lock:
                st = {'folders': {k: {kk: vv for kk, vv in v.items()} for k, v in FOLDERS.items()},
                      'items': [{'id': k, 'folder': v['folder'], 'subject': str(parsed(v)['subject']), 'read': v['read'], 'flag': v['flag'], 'importance': v['importance'], 'categories': v['categories']} for k, v in ITEMS.items()],
                      'sent': [email.message_from_bytes(m, policy=email.policy.default).as_string() for m in SENT], 'stats': STATS}
            return self.send(200, json.dumps(st, ensure_ascii=False), 'application/json')
        self.send(404, 'not found', 'text/plain')


STATS = {'auth': [], 'ops': []}

if __name__ == '__main__':
    cert, key = sys.argv[1], sys.argv[2]
    port = int(sys.argv[3]) if len(sys.argv) > 3 else 8445
    srv = ThreadingHTTPServer(('localhost', port), H)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(cert, key)
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    srv.serve_forever()
