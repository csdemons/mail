"""Имитация Nextcloud (Login Flow v2, Deck API v1.0, Talk API v4/v1) для тестов.
Форматы ответов — по официальной документации Deck и Talk.
python3 tests/servers/nextcloud_mock.py <cert> <key> [port=8443]

Тестовые ручки:
  POST /test/inject?token=<room>&text=...  — сообщение «от коллеги» (будит long-poll);
                                             &read=1 — коллега сначала прочитал всё (для «✓✓»)
  GET  /test/state                         — состояние для проверок
"""
import json, ssl, sys, threading, time, base64, itertools
import re as _re_mod, urllib.parse
_re_fb = _re_mod.compile(r"ATTENDEE[^:]*:mailto:([^\r\n]+)")
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import unquote, urlparse, parse_qs

LOGIN, APP_PASSWORD, UID = 'test@corp.test', 'app-pass-123', 'test'
lock = threading.Condition()
ids = itertools.count(100)


def png(w, h, color):
    import zlib, struct
    raw = b''.join(b'\x00' + bytes(color) * w for _ in range(h))
    ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b'')


PHOTO = png(64, 48, (30, 150, 90))
UPDATE_BLOB = b'fake package for update test\n' * 4000

state = {
    'dirs': {'/', '/Документы', '/Talk'},
    'dir_ids': {},
    'shares': [],
    'files': {'/Документы/Регламент.txt': {'id': 4242, 'data': 'Регламент работ\n'.encode()},
              '/Документы/План.docx': {'id': 4243, 'data': b'PK fake docx'},
              '/Talk/Фото с объекта.png': {'id': 4300, 'data': PHOTO}},
    'direct': {},
    'polls': 0,
    'revoked': False,
    'boards': [
        {'id': 1, 'title': 'УИТ — задачи', 'color': '0082c9', 'archived': False, 'deletedAt': 0,
         'owner': {'uid': UID, 'displayname': 'Тестовый Пользователь'},
         'permissions': {'PERMISSION_READ': True, 'PERMISSION_EDIT': True, 'PERMISSION_MANAGE': True, 'PERMISSION_SHARE': True}},
        {'id': 2, 'title': 'Объявления', 'color': 'e9322d', 'archived': False, 'deletedAt': 0,
         'owner': {'uid': 'admin', 'displayname': 'Администратор'},
         'permissions': {'PERMISSION_READ': True, 'PERMISSION_EDIT': False, 'PERMISSION_MANAGE': False, 'PERMISSION_SHARE': False}},
    ],
    'stacks': {
        1: [{'id': 10, 'title': 'К выполнению', 'boardId': 1, 'order': 0, 'deletedAt': 0},
            {'id': 11, 'title': 'В работе', 'boardId': 1, 'order': 1, 'deletedAt': 0},
            {'id': 12, 'title': 'Готово', 'boardId': 1, 'order': 2, 'deletedAt': 0}],
        2: [{'id': 20, 'title': 'Новости', 'boardId': 2, 'order': 0, 'deletedAt': 0}],
    },
    'cards': [
        {'id': 1, 'stackId': 10, 'title': 'Обновить RuPost', 'description': 'Окно работ в среду\n\n- [x] Бэкап\n- [ ] Обновить пакеты\n- [ ] Проверить почту', 'type': 'plain', 'order': 0,
         'duedate': '2026-10-30T00:00:00+00:00', 'done': None, 'archived': False, 'owner': {'uid': UID, 'displayname': 'Тестовый'},
         'labels': [{'id': 1, 'title': 'Срочно', 'color': 'e9322d', 'boardId': 1}],
         'assignedUsers': [{'participant': {'uid': 'ivanov', 'displayname': 'Иванов Иван'}}], 'commentsUnread': 1, 'attachmentCount': 0},
        {'id': 2, 'stackId': 11, 'title': 'Настроить брокер X2Go', 'description': '', 'type': 'plain', 'order': 0,
         'duedate': None, 'done': None, 'archived': False, 'owner': UID, 'labels': [], 'assignedUsers': [{'participant': {'uid': UID, 'displayname': 'Тестовый Пользователь'}}], 'commentsUnread': 0},
        {'id': 3, 'stackId': 20, 'title': 'Плановые работы 1 ноября', 'description': '', 'type': 'plain', 'order': 0,
         'duedate': None, 'done': None, 'archived': False, 'owner': 'admin', 'labels': [], 'assignedUsers': [], 'commentsUnread': 0},
    ],
    'board_labels': {1: [{'id': 1, 'title': 'Срочно', 'color': 'e9322d', 'boardId': 1}, {'id': 2, 'title': 'Сеть', 'color': '317cc9', 'boardId': 1}]},
    'comments': {1: [{'id': 501, 'objectId': 1, 'message': 'Согласовано с ИБ', 'actorId': 'ivanov', 'actorType': 'users',
                      'actorDisplayName': 'Иванов Иван', 'creationDateTime': '2026-10-26T09:00:00+00:00'}]},
    'attachments': {1: [{'id': 77, 'cardId': 1, 'type': 'file', 'data': 'regl.pdf', 'deletedAt': 0,
                         'extendedData': {'filesize': 2048, 'mimetype': 'application/pdf', 'fileid': 9001, 'info': {'filename': 'Регламент', 'extension': 'pdf'}}}]},
    'rooms': [
        {'token': 'abc123', 'type': 2, 'name': 'uit', 'displayName': 'УИТ — общий', 'readOnly': 0},
        {'token': 'dm-ivanov', 'type': 1, 'name': 'ivanov', 'displayName': 'Иванов Иван', 'readOnly': 0},
    ],
    'messages': {
        'abc123': [
            {'id': 1, 'actorType': 'users', 'actorId': 'ivanov', 'actorDisplayName': 'Иванов Иван', 'timestamp': 1793000000,
             'message': 'Коллеги, {mention-user1}, обновление в среду', 'messageType': 'comment', 'systemMessage': '',
             'messageParameters': {'mention-user1': {'type': 'user', 'id': UID, 'name': 'Тестовый Пользователь'}}},
            {'id': 2, 'actorType': 'users', 'actorId': 'petrova', 'actorDisplayName': 'Петрова Анна', 'timestamp': 1793000100,
             'message': 'План: {file}', 'messageType': 'comment', 'systemMessage': '',
             'messageParameters': {'file': {'type': 'file', 'id': '42', 'name': 'План работ.xlsx'}}},
            {'id': 3, 'actorType': 'users', 'actorId': 'petrova', 'actorDisplayName': 'Петрова Анна', 'timestamp': 1793000150,
             'message': 'reaction', 'messageType': 'system', 'systemMessage': 'reaction', 'messageParameters': []},
            {'id': 4, 'actorType': 'users', 'actorId': 'ivanov', 'actorDisplayName': 'Иванов Иван', 'timestamp': 1793000200,
             'message': '{file}', 'messageType': 'comment', 'systemMessage': '',
             'messageParameters': {'file': {'type': 'file', 'id': '4300', 'name': 'Фото с объекта.png', 'size': len(PHOTO), 'mimetype': 'image/png',
                                            'path': 'Talk/Фото с объекта.png', 'preview-available': 'yes'}}},
        ],
        'dm-ivanov': [],
    },
    'read': {'abc123': 0, 'dm-ivanov': 0},
    'talk_polls': {},
    # До какого сообщения беседу прочитали все участники (X-Chat-Last-Common-Read).
    'peer_read': {},
}


def push_update(token, kind, msg):
    """Как Talk: изменение сообщения приходит служебным сообщением с parent."""
    with lock:
        state['messages'][token].append({
            'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
            'timestamp': int(time.time()), 'message': kind, 'messageType': 'system', 'systemMessage': kind,
            'messageParameters': [], 'parent': dict(msg)})
        lock.notify_all()


USERS = [{'id': 'ivanov', 'label': 'Иванов Иван', 'subline': 'ivanov@inst.test'},
         {'id': 'petrova', 'label': 'Петрова Анна', 'subline': 'petrova@inst.test'},
         {'id': 'sidorov', 'label': 'Сидоров Олег', 'subline': 'sidorov@inst.test'}]
HIDDEN_USER = {'id': 'smirnov', 'label': 'Смирнов Пётр', 'subline': 'smirnov@inst.test'}


NAMES = {u['id']: u['label'] for u in USERS + [HIDDEN_USER]}
_att = iter(range(100, 10**6))


def attendee(uid, role=3, status='offline'):
    """Участник беседы, как его отдаёт Talk: роль 1 — владелец, 2 — модератор, 3 — участник."""
    return {'attendeeId': next(_att), 'actorType': 'users', 'actorId': uid,
            'displayName': NAMES.get(uid, 'Тестовый Пользователь' if uid == UID else uid),
            'participantType': role, 'status': status, 'statusMessage': ''}


def members(r):
    if 'members' not in r:
        if r['token'] == 'abc123':
            r['members'] = [attendee(UID, 1, 'online'), attendee('ivanov', 2, 'online'), attendee('petrova', 3, 'away')]
        elif r['type'] == 1:
            r['members'] = [attendee(UID, 1, 'online'), attendee(r['name'], 1, 'dnd')]
        else:
            r['members'] = [attendee(UID, 1, 'online')] + [attendee(u) for u in r.get('participants', [])]
    return r['members']


def expire(m, token):
    """Исчезающие сообщения: время удаления по настройке беседы."""
    r = next((x for x in state['rooms'] if x['token'] == token), None)
    if r and r.get('expiration'):
        m['expirationTimestamp'] = m['timestamp'] + r['expiration']


def poll_view(pl):
    voted = pl['votes'].get(UID, [])
    counts = {f'option-{i}': sum(1 for v in pl['votes'].values() if i in v) for i in range(len(pl['options']))}
    show = pl['resultMode'] == 0 or pl['status'] == 1
    return {'id': pl['id'], 'question': pl['question'], 'options': pl['options'], 'votes': counts if show else [],
            'numVoters': len(pl['votes']), 'votedSelf': voted, 'status': pl['status'], 'resultMode': pl['resultMode'],
            'maxVotes': pl['maxVotes'], 'actorType': 'users', 'actorId': pl['actorId'], 'actorDisplayName': pl['actorDisplayName']}


def room_view(r):
    me = next((m for m in members(r) if m['actorId'] == UID), None)
    msgs = [m for m in state['messages'][r['token']] if m['messageType'] != 'system']
    last = msgs[-1] if msgs else None
    unread = sum(1 for m in msgs if m['id'] > state['read'][r['token']] and m['actorId'] != UID)
    return dict({k: v for k, v in r.items() if k != 'members'}, unreadMessages=unread, unreadMention=False, isFavorite=bool(r.get('favorite')),
                participantType=me['participantType'] if me else 3, description=r.get('description', ''),
                notificationLevel=r.get('notify', 0), canLeaveConversation=True, messageExpiration=r.get('expiration', 0),
                canDeleteConversation=bool(me and me['participantType'] in (1, 2) and r['type'] != 1),
                lastCommonReadMessage=state['peer_read'].get(r['token'], 0),
                lastActivity=last['timestamp'] if last else 0, lastMessage=last or [],
                lastPinnedId=max([m['id'] for m in state['messages'][r['token']] if pin_active(m)] or [0]), hiddenPinnedId=state.get('pin_hidden', {}).get(r['token'], 0))


def pin_active(m):
    md = m.get('metaData') or {}
    return 'pinnedAt' in md and (not md.get('pinnedUntil') or md['pinnedUntil'] > time.time())


class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *a):
        pass

    def send(self, code, obj=None, headers=None):
        body = b'' if obj is None else json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def ocs(self, data, code=200, headers=None):
        self.send(code, {'ocs': {'meta': {'status': 'ok', 'statuscode': code, 'message': 'OK'}, 'data': data}}, headers)

    def peek_json(self):
        b = self.body()
        self._cached_body = b
        return b

    def body(self):
        if getattr(self, '_cached_body', None) is not None:
            b, self._cached_body = self._cached_body, None
            return b
        n = int(self.headers.get('Content-Length') or 0)
        raw = self.rfile.read(n) if n else b''
        if 'json' in (self.headers.get('Content-Type') or ''):
            return json.loads(raw or b'{}')
        return {k: v[0] for k, v in parse_qs(raw.decode()).items()}

    def authed(self):
        a = self.headers.get('Authorization', '')
        ok = a.startswith('Basic ') and base64.b64decode(a[6:]).decode() == f'{LOGIN}:{APP_PASSWORD}' and not state['revoked']
        if not ok:
            self.send(401, {'message': 'Unauthorized'})
        elif self.headers.get('OCS-APIRequest') != 'true':
            self.send(400, {'message': 'missing OCS-APIRequest'})
            return False
        return ok

    def route(self, method):
        u = urlparse(self.path)
        p, q = u.path, {k: v[0] for k, v in parse_qs(u.query).items()}
        base = f'https://localhost:{self.server.server_port}'

        # --- Без входа: отписка от рассылки (RFC 8058) и autoconfig почты
        if method == 'POST' and p.startswith('/unsub/'):
            state.setdefault('unsubscribed', []).append({'id': p.split('/')[-1], 'body': self.rfile.read(int(self.headers.get('Content-Length', 0))).decode()})
            return self.send(200, {})
        if method == 'GET' and p == '/autoconfig/mail/config-v1.1.xml':
            xml = ('<?xml version="1.0"?><clientConfig version="1.1"><emailProvider id="corp.test">'
                   '<incomingServer type="imap"><hostname>localhost</hostname><port>10993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>'
                   '<outgoingServer type="smtp"><hostname>localhost</hostname><port>465</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></outgoingServer>'
                   '</emailProvider></clientConfig>').encode()
            self.send_response(200); self.send_header('Content-Type', 'application/xml'); self.send_header('Content-Length', str(len(xml))); self.end_headers(); self.wfile.write(xml)
            return
        # --- Аватары: пользователя (PNG) и беседы Talk (SVG)
        if method == 'GET' and p.startswith('/index.php/avatar/'):
            import zlib, struct
            uid = unquote(p.split('/')[3])
            color = {'ivanov': (200, 60, 60), 'petrova': (60, 140, 200)}.get(uid, (90, 90, 90))
            raw = b''.join(b'\x00' + bytes(color) * 8 for _ in range(8))
            ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
            body = b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', 8, 8, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b'')
            state['avatars'] = state.get('avatars', 0) + 1
            self.send_response(200); self.send_header('Content-Type', 'image/png'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)
        # --- Уменьшенные копии файлов (core/preview по fileId)
        if method == 'GET' and p == '/index.php/core/preview':
            fid = q.get('fileId', '')
            f = next((v for v in state['files'].values() if str(v['id']) == fid), None)
            if f is None or not f['data'].startswith(b'\x89PNG'):
                return self.send(404)
            state['previews'] = state.get('previews', 0) + 1
            body = png(32, 24, (30, 150, 90))
            self.send_response(200); self.send_header('Content-Type', 'image/png'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)
        if method == 'GET' and p.startswith('/ocs/v2.php/apps/spreed/api/v1/room/') and p.endswith('/avatar'):
            body = b'<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#0a7"/></svg>'
            self.send_response(200); self.send_header('Content-Type', 'image/svg+xml'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)

        # --- Direct Editing: страница редактора по одноразовой ссылке (без входа)
        if method == 'GET' and p.startswith('/index.php/apps/eurooffice/direct/'):
            path = state['direct'].get(p.rsplit('/', 1)[1])
            if not path:
                return self.send(404, {})
            html = f'<!doctype html><meta charset="utf-8"><title>Euro-Office</title><h1 id="doc">Редактор: {path}</h1>'.encode()
            self.send_response(200); self.send_header('Content-Type', 'text/html; charset=utf-8'); self.send_header('Content-Length', str(len(html))); self.end_headers()
            return self.wfile.write(html)

        # --- Внешняя картинка для HTML-письма (зелёный PNG 40×30)
        if method == 'GET' and p == '/e2e/remote.png':
            import zlib, struct
            raw = b''.join(b'\x00' + bytes((30, 180, 60)) * 40 for _ in range(30))
            ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
            body = b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', 40, 30, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b'')
            state['remote_png'] = state.get('remote_png', 0) + 1
            self.send_response(200); self.send_header('Content-Type', 'image/png'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)

        # --- Login Flow v2 (без авторизации)
        if method == 'POST' and p == '/index.php/login/v2':
            return self.send(200, {'poll': {'token': 'poll-token', 'endpoint': f'{base}/login/v2/poll'},
                                   'login': f'{base}/login/v2/flow/abc'})
        if method == 'POST' and p == '/login/v2/poll':
            if self.body().get('token') != 'poll-token':
                return self.send(404)
            state['polls'] += 1
            state['revoked'] = False
            if state['polls'] < 3:  # пользователь ещё входит в браузере
                return self.send(404, [])
            return self.send(200, {'server': base, 'loginName': LOGIN, 'appPassword': APP_PASSWORD})

        # --- обновления (без авторизации, как GitHub Releases)
        if method == 'GET' and p in ('/updates/latest.json', '/updates/latest.json.sig'):
            import hashlib, subprocess, os, base64 as b64
            body = json.dumps({'version': '9.9.9', 'notes': '- Проверка обновлений\n- Ещё пункт', 'assets': {
                'test': {'name': 'rrcki-mail_9.9.9_amd64.deb', 'url': f'{base}/updates/rrcki-mail_9.9.9_amd64.deb',
                         'sha256': hashlib.sha256(UPDATE_BLOB).hexdigest(), 'size': len(UPDATE_BLOB)}}}, ensure_ascii=False).encode()
            if p.endswith('.sig'):
                # Подпись тестовым ключом (tests/servers/update-test-key.pem) — как в CI, через openssl.
                key = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'update-test-key.pem')
                import tempfile
                with tempfile.NamedTemporaryFile() as tf:
                    tf.write(body); tf.flush()
                    sig = subprocess.run(['openssl', 'pkeyutl', '-sign', '-inkey', key, '-rawin', '-in', tf.name], capture_output=True, check=True).stdout
                body = b64.b64encode(sig) + b'\n'
            self.send_response(200); self.send_header('Content-Type', 'application/octet-stream'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)
        if method == 'GET' and p == '/updates/rrcki-mail_9.9.9_amd64.deb':
            self.send_response(200); self.send_header('Content-Length', str(len(UPDATE_BLOB))); self.end_headers()
            self.wfile.write(UPDATE_BLOB); return

        # --- тестовые ручки
        if p == '/test/state':
            return self.send(200, {k: state[k] for k in ('polls', 'revoked', 'cards', 'stacks', 'comments', 'shares', 'boards')} | {k: state.get(k, d) for k, d in (('pins', []), ('unpins', []), ('reminders', {}), ('freebusy', []), ('pin_hidden', {}))} | {'dirs': sorted(state['dirs']), 'remote_png': state.get('remote_png', 0), 'previews': state.get('previews', 0), 'attachments': state['attachments'], 'unsubscribed': state.get('unsubscribed', [])} | {'messages': state['messages'], 'rooms': [room_view(r) for r in state['rooms']], 'members': {r['token']: members(r) for r in state['rooms']}, 'talk_polls': {str(k): poll_view(v) | {'allVotes': v['votes']} for k, v in state['talk_polls'].items()},
                             'files': {k: v['data'].decode(errors='replace') for k, v in state['files'].items()}})
        if method == 'POST' and p == '/test/set':
            for k in ('slow_send',):
                if k in q: state[k] = q[k]
            return self.send(200, {})
        if method == 'POST' and p == '/test/inject':
            with lock:
                if q.get('read') == '1' and state['messages'][q['token']]:
                    state['peer_read'][q['token']] = max(m['id'] for m in state['messages'][q['token']])
                if q.get('voice') == '1':
                    # Голосовое «от коллеги»: файл в /Talk и сообщение voice-message.
                    vpath = '/Talk/Голосовое Иванова.wav'
                    # Настоящий WAV (0,1 с тишины, 8 кГц, 8 бит) — проигрыватель должен его открыть.
                    pcm = b'\x80' * 800
                    wav = b'RIFF' + (36 + len(pcm)).to_bytes(4, 'little') + b'WAVEfmt ' + (16).to_bytes(4, 'little') + (1).to_bytes(2, 'little') + (1).to_bytes(2, 'little') \
                        + (8000).to_bytes(4, 'little') + (8000).to_bytes(4, 'little') + (1).to_bytes(2, 'little') + (8).to_bytes(2, 'little') + b'data' + len(pcm).to_bytes(4, 'little') + pcm
                    state['files'][vpath] = {'id': next(ids), 'data': wav}
                    vf = state['files'][vpath]
                    state['messages'][q['token']].append({
                        'id': next(ids), 'actorType': 'users', 'actorId': 'ivanov', 'actorDisplayName': 'Иванов Иван',
                        'timestamp': int(time.time()), 'message': '{file}', 'messageType': 'voice-message', 'systemMessage': '',
                        'messageParameters': {'file': {'type': 'file', 'id': str(vf['id']), 'name': 'Голосовое Иванова.wav', 'size': len(vf['data']), 'mimetype': 'audio/wav', 'path': vpath.lstrip('/')}}})
                else:
                    state['messages'][q['token']].append({
                        'id': next(ids), 'actorType': 'users', 'actorId': 'ivanov', 'actorDisplayName': 'Иванов Иван',
                        'timestamp': int(time.time()), 'message': q['text'], 'messageType': 'comment', 'systemMessage': '',
                        'messageParameters': []})
                lock.notify_all()
            return self.send(200, {})

        if method == 'POST' and p == '/test/poll_vote':
            # Голос «коллеги» за вариант опроса.
            pl = state['talk_polls'][int(q['id'])]
            pl['votes'][q.get('user', 'ivanov')] = [int(x) for x in q['options'].split(',')]
            return self.send(200, {})
        if not self.authed():
            return
        if p.startswith('/ocs/v2.php/apps/files/api/v1/directEditing'):
            mimes = {'docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
                     'xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
                     'pptx': 'application/vnd.openxmlformats-officedocument.presentationml.presentation'}
            if method == 'GET':
                return self.ocs({'editors': {'eurooffice': {'id': 'eurooffice', 'name': 'Euro-Office', 'mimetypes': list(mimes.values()), 'optionalMimetypes': [], 'secure': False}},
                                 'creators': {f'eurooffice_{e}': {'id': f'eurooffice_{e}', 'editor': 'eurooffice', 'name': e, 'extension': e, 'templates': False, 'mimetype': m} for e, m in mimes.items()}})
            b = self.body()
            path = b.get('path', '')
            if p.endswith('/create'):
                state['files'][path] = {'id': next(ids), 'data': b'PK new ' + path.encode()}
            elif path not in state['files']:
                return self.ocs({}, 404)
            tok = f'tok{next(ids)}'
            state['direct'][tok] = path
            return self.ocs({'url': f'{base}/index.php/apps/eurooffice/direct/{tok}'})
        if method == 'GET' and p.startswith('/ocs/v2.php/profile/'):
            uid = urllib.parse.unquote(p.rsplit('/', 1)[1])
            u = next((x for x in USERS if x['id'] == uid), None)
            if not u: return self.send(404)
            extra = {'ivanov': {'email': 'ivanov@inst.test', 'phone': '+7 495 000-00-01', 'organisation': 'НИЦ КИ, УИТ', 'role': 'Ведущий инженер'}}.get(uid, {})
            return self.ocs({'userId': uid, 'displayname': u['label'], 'address': None, 'headline': None, **extra})
        if method == 'GET' and p.startswith('/ocs/v2.php/apps/user_status/api/v1/statuses/'):
            return self.ocs({'userId': p.rsplit('/', 1)[1], 'status': 'away', 'icon': '🌴', 'message': 'В отпуске до пятницы'})
        if method == 'POST' and p == f'/remote.php/dav/calendars/{UID}/outbox/':
            n = int(self.headers.get('Content-Length', 0)); req = self.rfile.read(n).decode()
            state.setdefault('freebusy', []).append(req)
            att = _re_fb.search(req)
            who = att.group(1) if att else ''
            import datetime as _dt
            day = _dt.datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)
            while day.weekday() >= 5: day += _dt.timedelta(days=1)
            def z(h, m=0): return (day + _dt.timedelta(hours=h, minutes=m)).astimezone(_dt.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
            if who == 'ivanov@inst.test':
                status, data = '2.0;Success', f'BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VFREEBUSY\r\nFREEBUSY:{z(10)}/{z(11, 30)}\r\nFREEBUSY;FBTYPE=BUSY-TENTATIVE:{z(14)}/PT1H\r\nEND:VFREEBUSY\r\nEND:VCALENDAR\r\n'
            else:
                status, data = '3.7;Invalid calendar user', ''
            body = f'<?xml version="1.0"?><cal:schedule-response xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav"><cal:response><cal:recipient><d:href>mailto:{who}</d:href></cal:recipient><cal:request-status>{status}</cal:request-status><cal:calendar-data>{data}</cal:calendar-data></cal:response></cal:schedule-response>'.encode()
            self.send_response(200); self.send_header('Content-Type', 'application/xml; charset=utf-8'); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body); return
        if method == 'GET' and p == '/ocs/v1.php/cloud/user':
            return self.ocs({'id': UID, 'displayname': 'Тестовый Пользователь', 'email': LOGIN})
        if method == 'DELETE' and p == '/ocs/v2.php/core/apppassword':
            state['revoked'] = True
            return self.ocs([])

        # --- Deck
        D = '/index.php/apps/deck/api/v1.0/boards'
        if p == D and method == 'GET':
            return self.send(200, state['boards'])
        if p == D and method == 'POST':
            b = self.body()
            nb = {'id': next(ids), 'title': b['title'], 'color': b.get('color', '0082c9'), 'archived': False, 'deletedAt': 0,
                  'owner': {'uid': UID, 'displayname': 'Тестовый Пользователь'},
                  'permissions': {'PERMISSION_READ': True, 'PERMISSION_EDIT': True, 'PERMISSION_MANAGE': True, 'PERMISSION_SHARE': True}}
            state['boards'].append(nb); state['stacks'][nb['id']] = []
            return self.send(200, nb)
        # Комментарии (OCS) и вложения (API 1.1)
        CM = '/ocs/v2.php/apps/deck/api/v1.0/cards/'
        if p.startswith(CM) and '/comments' in p:
            rest = p[len(CM):].split('/')
            card_id = int(rest[0]); lst = state['comments'].setdefault(card_id, [])
            if method == 'GET':
                return self.ocs(list(reversed(lst)))
            if method == 'POST':
                b = self.body()
                c = {'id': next(ids), 'objectId': card_id, 'message': b['message'], 'actorId': UID, 'actorType': 'users',
                     'actorDisplayName': 'Тестовый Пользователь', 'creationDateTime': time.strftime('%Y-%m-%dT%H:%M:%S+00:00', time.gmtime())}
                lst.append(c); return self.ocs(c)
            if method == 'DELETE' and len(rest) == 3:
                state['comments'][card_id] = [c for c in lst if c['id'] != int(rest[2])]
                return self.ocs([])
        A = '/index.php/apps/deck/api/v1.1/boards/'
        if p.startswith(A) and p.endswith('/attachments') and method == 'GET':
            card_id = int(p[len(A):].split('/')[4])
            return self.send(200, state['attachments'].get(card_id, []))
        if p.startswith(A) and p.endswith('/attachments') and method == 'POST':
            # multipart/form-data: type=file и сам файл (как перетаскивание в веб-Deck)
            card_id = int(p[len(A):].split('/')[4])
            boundary = (self.headers.get('Content-Type') or '').split('boundary=')[-1].strip('"').encode()
            raw = self.rfile.read(int(self.headers.get('Content-Length') or 0))
            fields, name, data = {}, None, None
            for part in raw.split(b'--' + boundary):
                head, _, body = part.partition(b'\r\n\r\n')
                body = body[:-2] if body.endswith(b'\r\n') else body
                disp = head.decode(errors='replace')
                if 'name="file"' in disp:
                    name = disp.split('filename="', 1)[1].split('"', 1)[0]; data = body
                elif 'name="type"' in disp:
                    fields['type'] = body.decode()
            if fields.get('type') != 'file' or not name:
                return self.send(400, {'message': 'bad upload'})
            stem, _, ext = name.rpartition('.')
            a = {'id': next(ids), 'cardId': card_id, 'type': 'file', 'data': name, 'deletedAt': 0,
                 'extendedData': {'filesize': len(data), 'mimetype': 'text/plain', 'fileid': next(ids), 'info': {'filename': stem or ext, 'extension': ext if stem else ''}}}
            state['attachments'].setdefault(card_id, []).append(a)
            return self.send(200, a)
        if p.startswith(A) and '/attachments/' in p and method == 'DELETE':
            rest = p[len(A):].split('/')
            card_id, att = int(rest[4]), int(rest[6])
            if q.get('type') not in ('file', 'deck_file'):
                return self.send(400, {'message': 'type missing'})
            state['attachments'][card_id] = [x for x in state['attachments'].get(card_id, []) if x['id'] != att]
            return self.send(200, {})
        # История карточки (приложение Activity)
        if p == '/ocs/v2.php/apps/activity/api/v2/activity/filter' and method == 'GET':
            if q.get('object_type') != 'deck_card':
                return self.send(404, {'message': 'not found'})
            cid = q.get('object_id')
            items = [{'activity_id': 9000 + i, 'user': 'ivanov', 'subject': text, 'datetime': '2026-10-26T09:0%d:00+00:00' % i,
                      'subject_rich': ['', {'user': {'type': 'user', 'id': 'ivanov', 'name': 'Иванов Иван'}}]}
                     for i, text in enumerate([f'Иванов Иван создал(а) карточку {cid}', 'Иванов Иван добавил(а) метку «Срочно»'])]
            return self.ocs(items)
        parts = p[len(D):].strip('/').split('/') if p.startswith(D + '/') else []
        if len(parts) == 1 and method == 'GET':
            b = next(x for x in state['boards'] if x['id'] == int(parts[0]))
            return self.send(200, dict(b, labels=state['board_labels'].get(b['id'], []),
                                       users=[{'uid': 'ivanov', 'displayname': 'Иванов Иван'}],
                                       acl=[{'type': 0, 'participant': {'uid': 'petrova', 'displayname': 'Петрова Анна'}}]))
        if parts:
            board = int(parts[0])
            perms = next(b for b in state['boards'] if b['id'] == board)['permissions']
            if method != 'GET' and not perms['PERMISSION_EDIT']:
                return self.send(403, {'status': 403, 'message': 'Permission denied'})
            if parts[1:] == ['stacks'] and method == 'GET':
                out = []
                for s in state['stacks'][board]:
                    cards = sorted([c for c in state['cards'] if c['stackId'] == s['id'] and not c.get('archived')], key=lambda c: c['order'])
                    out.append(dict(s, cards=cards))
                return self.send(200, out)
            if parts[1:] == ['stacks'] and method == 'POST':
                b = self.body()
                s = {'id': next(ids), 'title': b['title'], 'boardId': board, 'order': b.get('order', 999), 'deletedAt': 0}
                state['stacks'][board].append(s)
                return self.send(200, s)
            if len(parts) >= 4 and parts[1] == 'stacks' and parts[3] == 'cards':
                stack = int(parts[2])
                if len(parts) == 4 and method == 'POST':
                    b = self.body()
                    for f in ('title', 'type', 'order'):
                        if f not in b:
                            return self.send(400, {'message': f'{f} missing'})
                    c = {'id': next(ids), 'stackId': stack, 'title': b['title'], 'description': b.get('description', ''),
                         'type': b['type'], 'order': b['order'], 'duedate': b.get('duedate'), 'done': None, 'archived': False,
                         'owner': {'uid': UID, 'displayname': 'Тестовый'}, 'labels': [], 'assignedUsers': None, 'commentsUnread': 0, 'createdAt': int(time.time()), 'lastModified': int(time.time())}
                    state['cards'].append(c)
                    return self.send(200, c)
                card = next((c for c in state['cards'] if c['id'] == int(parts[4])), None) if len(parts) >= 5 else None
                if card is None:
                    return self.send(404, {'message': 'not found'})
                if len(parts) == 5 and method == 'GET':
                    return self.send(200, card)
                if len(parts) == 5 and method == 'PUT':
                    b = self.body()
                    for f in ('title', 'type', 'owner', 'order', 'description'):
                        if f not in b:
                            return self.send(400, {'message': f'{f} missing'})
                    card.update(title=b['title'], description=b['description'], duedate=b.get('duedate'), done=b.get('done'))
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] in ('assignUser', 'unassignUser') and method == 'PUT':
                    uid = self.body()['userId']
                    users = card.get('assignedUsers') or []
                    users = [u for u in users if u['participant']['uid'] != uid]
                    if parts[5] == 'assignUser':
                        name = {'ivanov': 'Иванов Иван', 'petrova': 'Петрова Анна', UID: 'Тестовый Пользователь'}.get(uid, uid)
                        users.append({'participant': {'uid': uid, 'displayname': name}})
                    card['assignedUsers'] = users
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] in ('assignLabel', 'removeLabel') and method == 'PUT':
                    lid = int(self.body()['labelId'])
                    lab = next(l for l in state['board_labels'][board] if l['id'] == lid)
                    card['labels'] = [l for l in card.get('labels', []) if l['id'] != lid] + ([lab] if parts[5] == 'assignLabel' else [])
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] in ('archive', 'unarchive') and method == 'PUT':
                    card['archived'] = parts[5] == 'archive'
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] == 'reorder' and method == 'PUT':
                    b = self.body()
                    # Как в Nextcloud: параметры пути важнее тела запроса.
                    # Как в Nextcloud (CardService::reorder): карточка встаёт на место `order`
                    # среди карточек колонки, остальные нумеруются заново.
                    order = int(b['order'])
                    card.update(stackId=stack)
                    others = sorted([c for c in state['cards'] if c['stackId'] == stack and c is not card], key=lambda c: c['order'])
                    others.insert(min(order, len(others)), card)
                    for i, c in enumerate(others): c['order'] = i
                    return self.send(200, others)
                if len(parts) == 5 and method == 'DELETE':
                    state['cards'].remove(card)
                    return self.send(200, card)

        # --- Поиск файлов по имени (WebDAV SEARCH, как в Nextcloud)
        if method == 'SEARCH' and p.rstrip('/') == '/remote.php/dav':
            import re as _re
            body = self.rfile.read(int(self.headers.get('Content-Length') or 0)).decode()
            state.setdefault('dav_search', []).append(body)
            m = _re.search(r'<d:literal>%(.*?)%</d:literal>', body)
            term = (m.group(1) if m else '').replace('&lt;', '<').replace('&gt;', '>').replace('&amp;', '&').lower()
            limit = int((_re.search(r'<d:nresults>(\d+)</d:nresults>', body) or [0, 50])[1])
            hits = [(f, False) for f in state['files'] if term in f.rsplit('/', 1)[-1].lower()]
            hits += [(d_, True) for d_ in state['dirs'] if d_ != '/' and term in d_.rsplit('/', 1)[-1].lower()]
            prefix = f'/remote.php/dav/files/{UID}/'
            out = ('<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">' + ''.join(self.dav_entry(x, isd, prefix) for x, isd in hits[:limit]) + '</d:multistatus>').encode()
            self.send_response(207); self.send_header('Content-Type', 'application/xml; charset=utf-8')
            self.send_header('Content-Length', str(len(out))); self.end_headers(); self.wfile.write(out)
            return
        # --- Файлы (WebDAV) и публикация в беседу
        F = f'/remote.php/dav/files/{UID}/'
        if p.startswith(F):
            path = '/' + unquote(p[len(F):]).strip('/')
            if method == 'MKCOL':
                if path in state['dirs']:
                    return self.send(405)
                state['dirs'].add(path)
                return self.send(201)
            if method == 'PROPFIND':
                return self.propfind(path, F)
            if method == 'GET':
                f = state['files'].get(path)
                if f is None:
                    return self.send(404)
                self.send_response(200); self.send_header('Content-Length', str(len(f['data']))); self.end_headers()
                self.wfile.write(f['data']); return
            if method == 'DELETE':
                if path in state['dirs']:
                    state['dirs'] = {d for d in state['dirs'] if d != path and not d.startswith(path + '/')}
                    for k in [k for k in state['files'] if k.startswith(path + '/')]: del state['files'][k]
                    return self.send(204)
                if state['files'].pop(path, None) is None:
                    return self.send(404)
                return self.send(204)
            if method == 'MOVE':
                dest = '/' + unquote(urlparse(self.headers['Destination']).path[len(F):]).strip('/')
                if dest in state['files'] or dest in state['dirs']:
                    return self.send(412)
                if path in state['files']:
                    state['files'][dest] = state['files'].pop(path)
                elif path in state['dirs']:
                    state['dirs'] = {dest + d[len(path):] if d == path or d.startswith(path + '/') else d for d in state['dirs']}
                    for k in [k for k in state['files'] if k.startswith(path + '/')]: state['files'][dest + k[len(path):]] = state['files'].pop(k)
                else:
                    return self.send(404)
                return self.send(201)
            if method == 'PUT':
                if path.rsplit('/', 1)[0] not in state['dirs'] and path.rsplit('/', 1)[0] != '':
                    return self.send(409)
                n = int(self.headers.get('Content-Length') or 0)
                data = self.rfile.read(n)
                if path in state['files'] and self.headers.get('If-None-Match') == '*':
                    return self.send(412)
                state['files'][path] = {'id': next(ids), 'data': data}
                return self.send(201)
        if method == 'GET' and p == '/ocs/v2.php/cloud/capabilities':
            return self.ocs({'capabilities': {'spreed': {'config': {'attachments': {'allowed': True, 'folder': '/Talk'}}}}})
        SH = '/ocs/v2.php/apps/files_sharing/api/v1/shares'
        if method == 'GET' and p == SH:
            return self.ocs([x for x in state['shares'] if x['path'] == q.get('path')])
        if method == 'DELETE' and p.startswith(SH + '/'):
            sid = p[len(SH) + 1:]
            state['shares'] = [x for x in state['shares'] if x['id'] != sid]
            return self.ocs([])
        if method == 'POST' and p == SH and int(self.peek_json().get('shareType', -1)) in (0, 3):
            b = self.body()
            if b['path'] not in state['files'] and b['path'] not in state['dirs']:
                return self.send(404, {'ocs': {'meta': {'status': 'failure', 'statuscode': 404, 'message': 'Wrong path'}}})
            sid = str(next(ids))
            if int(b['shareType']) == 3:
                sh = {'id': sid, 'share_type': 3, 'path': b['path'], 'share_with': None, 'permissions': 1,
                      'url': f'{base}/s/tok{sid}', 'token': f'tok{sid}', 'expiration': (b.get('expireDate') or '') and b['expireDate'] + ' 00:00:00',
                      'password': 'hash' if b.get('password') else None}
            else:
                u = next((x for x in USERS if x['id'] == b['shareWith']), None)
                if not u: return self.send(404, {})
                sh = {'id': sid, 'share_type': 0, 'path': b['path'], 'share_with': u['id'], 'share_with_displayname': u['label'],
                      'permissions': int(b.get('permissions', 1)), 'url': ''}
            state['shares'].append(sh)
            return self.ocs(sh)
        if method == 'POST' and p == SH:
            b = self.body()
            f = state['files'].get(b.get('path'))
            if int(b.get('shareType', -1)) != 10 or f is None or b.get('shareWith') not in state['messages']:
                return self.send(404, {'message': 'bad share'})
            meta = json.loads(b.get('talkMetaData') or '{}')
            name = b['path'].rsplit('/', 1)[1]
            with lock:
                m = {'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
                     'timestamp': int(time.time()), 'message': meta.get('caption') or '{file}', 'messageType': meta.get('messageType', 'comment'), 'systemMessage': '',
                     'silent': bool(meta.get('silent')),
                     'messageParameters': {'file': {'type': 'file', 'id': str(f['id']), 'name': name, 'size': len(f['data']),
                                                    'mimetype': __import__('mimetypes').guess_type(name)[0] or 'text/plain', 'path': b['path']}}}
                expire(m, b['shareWith'])
                state['messages'][b['shareWith']].append(m)
                lock.notify_all()
            return self.ocs({'id': next(ids)})

        # --- Talk
        if method == 'GET' and p == '/ocs/v2.php/apps/spreed/api/v4/room':
            return self.ocs([room_view(r) for r in state['rooms']])
        if method == 'POST' and p == '/ocs/v2.php/apps/spreed/api/v4/room':
            b = self.body()
            if int(b.get('roomType', 0)) == 1:
                u = next((x for x in USERS + [HIDDEN_USER] if x['id'] == b.get('invite')), None)
                if not u:
                    return self.send(404, {})
                r = next((x for x in state['rooms'] if x['type'] == 1 and x['name'] == u['id']), None)
                if not r:
                    r = {'token': f"dm-{u['id']}", 'type': 1, 'name': u['id'], 'displayName': u['label'], 'readOnly': 0}
                    state['rooms'].append(r); state['messages'][r['token']] = []; state['read'][r['token']] = 0
            else:
                tok = f'grp{next(ids)}'
                r = {'token': tok, 'type': 2, 'name': b['roomName'], 'displayName': b['roomName'], 'readOnly': 0, 'participants': []}
                state['rooms'].append(r); state['messages'][tok] = []; state['read'][tok] = 0
            return self.ocs(room_view(r), 201)
        if method in ('POST', 'DELETE') and p.startswith('/ocs/v2.php/apps/spreed/api/v4/room/') and p.endswith('/favorite'):
            tok = p.split('/')[-2]
            r = next((x for x in state['rooms'] if x['token'] == tok), None)
            if not r:
                return self.send(404, {})
            r['favorite'] = method == 'POST'
            return self.ocs({})
        if method == 'GET' and p == '/ocs/v2.php/search/providers/talk-message/search':
            # Общий поиск Nextcloud по сообщениям всех бесед.
            term = q.get('term', '').lower()
            hits = [(tok, m) for tok, msgs in state['messages'].items() for m in msgs if m.get('systemMessage', '') == '' and term in m['message'].lower()]
            hits.sort(key=lambda x: -x[1]['timestamp'])
            return self.ocs({'name': 'Сообщения', 'isPaginated': False, 'cursor': None, 'entries': [
                {'title': m['actorDisplayName'], 'subline': m['message'], 'resourceUrl': f"/call/{tok}#message_{m['id']}",
                 'attributes': {'conversation': tok, 'messageId': str(m['id']), 'actorType': 'users', 'actorId': m['actorId'], 'timestamp': str(m['timestamp'])}}
                for tok, m in hits[:int(q.get('limit', 20))]]})
        if method == 'GET' and p == '/ocs/v2.php/search/providers/talk-message-current/search':
            # Общий поиск Nextcloud по сообщениям текущей беседы (from=/call/<token>).
            token = q.get('from', '').rsplit('/', 1)[-1]
            term = q.get('term', '').lower()
            hits = [m for m in state['messages'].get(token, []) if m.get('systemMessage', '') == '' and term in m['message'].lower()]
            return self.ocs({'name': 'Сообщения', 'isPaginated': False, 'cursor': None, 'entries': [
                {'title': m['actorDisplayName'], 'subline': m['message'], 'resourceUrl': f"/call/{token}#message_{m['id']}",
                 'attributes': {'conversation': token, 'messageId': str(m['id']), 'actorType': 'users', 'actorId': m['actorId'], 'timestamp': str(m['timestamp'])}}
                for m in reversed(hits)]})
        PL = '/ocs/v2.php/apps/spreed/api/v1/poll/'
        if p.startswith(PL):
            tok, *rest = p[len(PL):].split('/')
            if tok not in state['messages']:
                return self.send(404)
            if method == 'POST' and not rest:
                b = self.body()
                pid = next(ids)
                pl = {'id': pid, 'question': b['question'], 'options': list(b['options']), 'resultMode': int(b.get('resultMode', 0)),
                      'maxVotes': int(b.get('maxVotes', 0)), 'status': 0, 'votes': {}, 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь'}
                state['talk_polls'][pid] = pl
                with lock:
                    m = {'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
                         'timestamp': int(time.time()), 'message': '{object}', 'messageType': 'comment', 'systemMessage': '',
                         'messageParameters': {'object': {'type': 'talk-poll', 'id': str(pid), 'name': b['question']}}}
                    state['messages'][tok].append(m)
                    lock.notify_all()
                return self.ocs(poll_view(pl), 201)
            pl = state['talk_polls'].get(int(rest[0])) if rest else None
            if not pl:
                return self.send(404)
            if method == 'GET':
                return self.ocs(poll_view(pl))
            if method == 'POST':
                if pl['status'] == 1:
                    return self.ocs({}, 400)
                opts = [int(x) for x in self.body().get('optionIds', [])]
                if pl['maxVotes'] and len(opts) > pl['maxVotes']:
                    return self.ocs({}, 400)
                if opts: pl['votes'][UID] = opts
                else: pl['votes'].pop(UID, None)
                return self.ocs(poll_view(pl))
            if method == 'DELETE':
                pl['status'] = 1
                return self.ocs(poll_view(pl))
        if method == 'GET' and p == '/ocs/v2.php/references/resolve':
            ref = q.get('reference', '')
            if 'example.org' not in ref:
                return self.ocs({'references': {ref: None}})
            og = {'id': ref, 'name': 'Пример: регламент выпуска', 'description': 'Порядок выпуска новых версий программ института.',
                  'thumb': f'{base}/index.php/core/references/preview/e2e', 'link': ref}
            return self.ocs({'references': {ref: {'richObjectType': 'open-graph', 'openGraphObject': og, 'accessible': True}}})
        if method == 'GET' and p == '/index.php/core/references/preview/e2e':
            self.send_response(200); self.send_header('Content-Type', 'image/png'); self.send_header('Content-Length', str(len(PHOTO))); self.end_headers()
            self.wfile.write(PHOTO); return
        CH = '/ocs/v2.php/apps/spreed/api/v1/chat/'
        if method == 'GET' and p.startswith(CH) and p.endswith('/share'):
            tok = p[len(CH):].split('/')[0]
            kind = q.get('objectType', 'media')
            def kind_of(m):
                f = (m.get('messageParameters') or {}).get('file') if isinstance(m.get('messageParameters'), dict) else None
                if not f: return 'other' if m.get('messageParameters') and isinstance(m['messageParameters'], dict) and 'object' in m['messageParameters'] else None
                if m.get('messageType') == 'voice-message': return 'voice'
                mt = f.get('mimetype', '')
                return 'media' if mt.startswith(('image/', 'video/')) else 'audio' if mt.startswith('audio/') else 'file'
            if kind == 'pinned':
                return self.ocs({str(m['id']): m for m in state['messages'].get(tok, []) if pin_active(m)})
            items = {str(m['id']): m for m in state['messages'].get(tok, []) if kind_of(m) == kind}
            return self.ocs(items)
        # Закрепление (Talk 23) и напоминания о сообщениях.
        import re as _re2
        mm = _re2.match(r'^/ocs/v2\.php/apps/spreed/api/v1/chat/([a-z0-9]+)/(\d+)/(pin|pin/self|reminder)$', p)
        if mm and mm.group(1) in state['messages']:
            tok, mid, what = mm.group(1), int(mm.group(2)), mm.group(3)
            msg = next((m for m in state['messages'][tok] if m['id'] == mid), None)
            if msg is None: return self.send(404)
            if what == 'pin' and method == 'POST':
                until = int(self.body().get('pinUntil', 0))
                if until and until < time.time(): return self.send(400, {'ocs': {'meta': {'status': 'failure', 'statuscode': 400}, 'data': {'error': 'until'}}})
                msg['metaData'] = {'pinnedActorType': 'users', 'pinnedActorId': UID, 'pinnedActorDisplayName': 'Тестовый Пользователь', 'pinnedAt': int(time.time()), **({'pinnedUntil': until} if until else {})}
                state.setdefault('pins', []).append({'token': tok, 'id': mid, 'until': until})
                return self.ocs(msg)
            if what == 'pin' and method == 'DELETE':
                msg.pop('metaData', None); state.setdefault('unpins', []).append({'token': tok, 'id': mid}); return self.ocs(msg)
            if what == 'pin/self' and method == 'DELETE':
                state.setdefault('pin_hidden', {})[tok] = mid; return self.ocs(None)
            rem = state.setdefault('reminders', {})
            key = f'{tok}:{mid}'
            if method == 'POST':
                rem[key] = int(self.body()['timestamp']); return self.ocs({'userId': UID, 'token': tok, 'messageId': mid, 'timestamp': rem[key]}, 201)
            if method == 'GET':
                return self.ocs({'userId': UID, 'token': tok, 'messageId': mid, 'timestamp': rem[key]}) if key in rem else self.send(404, {'ocs': {'meta': {'status': 'failure', 'statuscode': 404}, 'data': []}})
            if method == 'DELETE':
                rem.pop(key, None); return self.ocs(None)
        RM = '/ocs/v2.php/apps/spreed/api/v4/room/'
        if p.startswith(RM) and p[len(RM):].split('/')[0] in [x['token'] for x in state['rooms']]:
            tok, *rest = p[len(RM):].split('/')
            r = next(x for x in state['rooms'] if x['token'] == tok)
            sub = '/'.join(rest)
            me = next((m for m in members(r) if m['actorId'] == UID), None)
            mod = bool(me and me['participantType'] in (1, 2))
            b = self.body() if method in ('POST', 'PUT', 'DELETE') else {}
            find = lambda: next((m for m in members(r) if m['attendeeId'] == int(b.get('attendeeId', q.get('attendeeId', 0)))), None)
            if method == 'GET' and sub == 'participants':
                return self.ocs(members(r))
            if method == 'POST' and sub == 'participants':
                uid = b['newParticipant']
                r.setdefault('participants', []).append(uid)
                if not any(m['actorId'] == uid for m in members(r)):
                    members(r).append(attendee(uid))
                return self.ocs({})
            if method == 'DELETE' and sub == 'participants/self':
                r['members'] = [m for m in members(r) if m['actorId'] != UID]
                state['rooms'].remove(r)
                return self.ocs({})
            if method == 'POST' and sub == 'message-expiration':
                if not mod: return self.ocs({}, 403)
                r['expiration'] = int(b.get('seconds', 0)); return self.ocs({})
            if method == 'POST' and sub == 'notify':
                r['notify'] = int(b.get('level', 0)); return self.ocs({})
            if not mod and (sub in ('attendees', 'moderators', 'description', 'read-only', 'public') or (sub == '' and method in ('PUT', 'DELETE'))):
                return self.ocs({}, 403)
            if method == 'DELETE' and sub == 'attendees':
                a = find()
                if not a: return self.ocs({}, 404)
                r['members'].remove(a); return self.ocs({})
            if sub == 'moderators':
                a = find()
                if not a: return self.ocs({}, 404)
                a['participantType'] = 2 if method == 'POST' else 3; return self.ocs({})
            if method == 'PUT' and sub == '':
                r['displayName'] = r['name'] = b['roomName']; return self.ocs({})
            if method == 'PUT' and sub == 'description':
                r['description'] = b.get('description', ''); return self.ocs({})
            if method == 'PUT' and sub == 'read-only':
                r['readOnly'] = int(b.get('state', 0)); return self.ocs({})
            if sub == 'public':
                r['type'] = 3 if method == 'POST' else 2; return self.ocs({})
            if method == 'DELETE' and sub == '':
                state['rooms'].remove(r); return self.ocs({})
        if method == 'GET' and p == '/ocs/v2.php/core/autocomplete/get':
            sq = q.get('search', '').lower()
            # Как при ограничении автодополнения: Смирнов находится только по точному e-mail.
            hidden = [HIDDEN_USER] if sq == HIDDEN_USER['subline'] else []
            return self.ocs([u for u in USERS if sq in (u['label'] + u['id']).lower()] + hidden)
        R = '/ocs/v2.php/apps/spreed/api/v1/reaction/'
        if p.startswith(R):
            token, mid = p[len(R):].split('/')
            msg = next(m for m in state['messages'][token] if m['id'] == int(mid))
            emoji = self.body().get('reaction') if method == 'POST' else unquote(q.get('reaction', ''))
            reactions = msg.setdefault('reactions', {}); mine = msg.setdefault('reactionsSelf', [])
            if method == 'POST' and emoji not in mine:
                reactions[emoji] = reactions.get(emoji, 0) + 1; mine.append(emoji)
            elif method == 'DELETE' and emoji in mine:
                reactions[emoji] -= 1; mine.remove(emoji)
                if reactions[emoji] <= 0: del reactions[emoji]
            push_update(token, 'reaction' if method == 'POST' else 'reaction_revoked', msg)
            return self.ocs(reactions, 201 if method == 'POST' else 200)
        T = '/ocs/v2.php/apps/spreed/api/v1/chat/'
        if p.startswith(T) and p.endswith('/mentions'):
            sq = q.get('search', '').lower()
            out = [{'id': 'all', 'label': 'УИТ — общий', 'source': 'calls', 'mentionId': 'all'}] if 'all'.startswith(sq) or not sq else []
            out += [{'id': u['id'], 'label': u['label'], 'source': 'users', 'mentionId': u['id']} for u in USERS if sq in u['label'].lower() or sq in u['id']]
            return self.ocs(out)
        if p.startswith(T) and p.endswith('/read') and method in ('POST', 'DELETE'):
            token = p[len(T):].split('/')[0]
            msgs = state['messages'][token]
            # «Непрочитано» — как в Talk: последнее сообщение снова непрочитанное.
            last = self.body().get('lastReadMessage') if method == 'POST' else None
            state['read'][token] = (int(last) if last is not None else max((m['id'] for m in msgs), default=0)) if method == 'POST' else max((m['id'] for m in msgs[:-1]), default=0)
            return self.ocs({})
        if p.startswith(T) and p[len(T):].count('/') == 1 and method in ('PUT', 'DELETE'):
            token, mid = p[len(T):].split('/')
            msg = next(m for m in state['messages'][token] if m['id'] == int(mid))
            if msg['actorId'] != UID:
                return self.send(403, {})
            if method == 'PUT':
                b = self.body()
                text, params = b['message'], {}
                import re as _re
                for i, mm in enumerate(_re.findall(r'@"([^"]+)"', text)):
                    u = next((x for x in USERS if x['id'] == mm), None)
                    if u:
                        params[f'mention-user{i}'] = {'type': 'user', 'id': u['id'], 'name': u['label'], 'mention-id': u['id']}
                        text = text.replace(f'@"{mm}"', '{' + f'mention-user{i}' + '}', 1)
                msg.update(message=text, messageParameters=params or [], lastEditTime=int(time.time()))
                push_update(token, 'message_edited', msg)
            else:
                msg.update(message='Message deleted by you', messageType='comment_deleted', messageParameters=[], reactions={}, reactionsSelf=[])
                push_update(token, 'message_deleted', msg)
            return self.ocs(msg)
        if p.startswith(T):
            token = p[len(T):]
            if token not in state['messages']:
                return self.send(404)
            if method == 'POST':
                b = self.body()
                if state.get('slow_send'): time.sleep(float(state['slow_send']))
                with lock:
                    import re as _re
                    text, params = b['message'], {}
                    for i, mm in enumerate(_re.findall(r'@"([^"]+)"', text)):
                        u = next((x for x in USERS if x['id'] == mm), None)
                        if u:
                            params[f'mention-user{i}'] = {'type': 'user', 'id': u['id'], 'name': u['label'], 'mention-id': u['id']}
                            text = text.replace(f'@"{mm}"', '{' + f'mention-user{i}' + '}', 1)
                    m = {'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
                         'timestamp': int(time.time()), 'message': text, 'messageType': 'comment', 'systemMessage': '',
                         'messageParameters': params or [], 'raw': b['message'], 'silent': bool(b.get('silent'))}
                    expire(m, token)
                    if b.get('replyTo'):
                        parent = next(x for x in state['messages'][token] if x['id'] == int(b['replyTo']))
                        m['parent'] = parent
                    state['messages'][token].append(m)
                    lock.notify_all()
                return self.ocs(m, 201)
            future = q.get('lookIntoFuture') == '1'
            last = int(q.get('lastKnownMessageId', 0))
            limit = int(q.get('limit', 100))
            with lock:
                if future:
                    deadline = time.time() + min(int(q.get('timeout', 30)), 60)
                    while not [m for m in state['messages'][token] if m['id'] > last] and time.time() < deadline:
                        lock.wait(deadline - time.time())
                    msgs = [m for m in state['messages'][token] if m['id'] > last][:limit]
                else:
                    older = [m for m in state['messages'][token] if not last or m['id'] < last]
                    msgs = list(reversed(older[-limit:]))  # история приходит от новых к старым
                if msgs and q.get('setReadMarker', '1') == '1':
                    state['read'][token] = max(state['read'][token], max(m['id'] for m in msgs))
            common = str(state['peer_read'].get(token, 0))
            if not msgs:
                self.send_response(304); self.send_header('Content-Length', '0'); self.send_header('X-Chat-Last-Common-Read', common); self.end_headers()
                return
            return self.ocs(msgs, 200, {'X-Chat-Last-Given': str(msgs[-1]['id']), 'X-Chat-Last-Common-Read': common})

        self.send(404, {'message': f'no route {method} {p}'})

    def do_GET(self): self.route('GET')
    def do_POST(self): self.route('POST')
    def do_PUT(self): self.route('PUT')
    def do_DELETE(self): self.route('DELETE')
    def do_MKCOL(self): self.route('MKCOL')
    def do_PROPFIND(self): self.route('PROPFIND')
    def do_SEARCH(self): self.route('SEARCH')
    def do_MOVE(self): self.route('MOVE')

    def dav_entry(self, pth, is_dir, prefix):
        from urllib.parse import quote
        href = prefix.rstrip('/') + quote(pth) + ('/' if is_dir and pth != '/' else '')
        fid = state['dir_ids'].setdefault(pth, next(ids)) if is_dir else state['files'][pth]['id']
        shared = any(x['path'] == pth for x in state['shares'])
        props = f'<oc:fileid>{fid}</oc:fileid><oc:permissions>RGDNVCK</oc:permissions><d:getlastmodified>Tue, 27 Oct 2026 10:00:00 GMT</d:getlastmodified>'
        props += '<oc:share-types>' + ('<oc:share-type>3</oc:share-type>' if shared else '') + '</oc:share-types>'
        if is_dir:
            size = sum(len(v['data']) for k, v in state['files'].items() if k.startswith(pth.rstrip('/') + '/'))
            props += f'<d:resourcetype><d:collection/></d:resourcetype><oc:size>{size}</oc:size>'
        else:
            props += f'<d:resourcetype/><d:getcontentlength>{len(state["files"][pth]["data"])}</d:getcontentlength><d:getcontenttype>text/plain</d:getcontenttype>'
        return f'<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>'

    def propfind(self, path, prefix):
        from urllib.parse import quote
        n = int(self.headers.get('Content-Length') or 0)
        if n: self.rfile.read(n)
        if path not in state['dirs'] and path not in state['files']:
            return self.send(404)
        def entry(pth, is_dir):
            return self.dav_entry(pth, is_dir, prefix)
        items = [entry(path, path in state['dirs'])]
        if path in state['dirs'] and self.headers.get('Depth') == '1':
            base_ = path.rstrip('/') + '/'
            children = {d for d in state['dirs'] if d.startswith(base_) and '/' not in d[len(base_):] and d != path}
            items += [entry(d, True) for d in sorted(children)]
            items += [entry(f, False) for f in sorted(state['files']) if f.startswith(base_) and '/' not in f[len(base_):]]
        body = ('<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">' + ''.join(items) + '</d:multistatus>').encode()
        self.send_response(207); self.send_header('Content-Type', 'application/xml; charset=utf-8')
        self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)


cert, key = sys.argv[1:3]
port = int(sys.argv[3]) if len(sys.argv) > 3 else 8443
srv = ThreadingHTTPServer(('127.0.0.1', port), H)
ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
ctx.load_cert_chain(cert, key)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print(f'Nextcloud mock on https://localhost:{port}', flush=True)
srv.serve_forever()
