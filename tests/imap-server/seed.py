import imaplib, ssl, base64, time, os
from email.message import EmailMessage
from email.mime.text import MIMEText
from email.header import Header
from email.utils import formatdate

def utf7(s):
    out, buf = [], ''
    def flush():
        nonlocal buf
        if buf:
            b = base64.b64encode(buf.encode('utf-16-be')).decode().rstrip('=').replace('/', ',')
            out.append('&' + b + '-'); buf = ''
    for ch in s:
        if 0x20 <= ord(ch) <= 0x7e:
            flush(); out.append('&-' if ch == '&' else ch)
        else:
            buf += ch
    flush(); return ''.join(out)

ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx)
M.login('test@rrcki.ru', 'Secret-123')
proj = utf7('Проекты/2026')
M.create('"%s"' % proj)

def append(folder, raw, flags='', days_ago=0):
    t = imaplib.Time2Internaldate(time.time() - days_ago * 86400)
    typ, _ = M.append(folder, flags, t, raw)
    assert typ == 'OK', typ

# 120 простых писем для постраничной загрузки
for i in range(120):
    m = MIMEText(f'Тестовое письмо номер {i}.\nВторая строка.', 'plain', 'utf-8')
    m['Subject'] = Header(f'Письмо №{i}', 'utf-8')
    m['From'] = 'Робот <robot@rrcki.ru>'; m['To'] = 'test@rrcki.ru'
    m['Date'] = formatdate(time.time() - (200 - i) * 3600, localtime=True)
    append('INBOX', m.as_bytes(), '(\\Seen)' if i % 3 else '', days_ago=(120 - i) / 10)

# windows-1251 + quoted-printable
m = MIMEText('Добрый день!\nПисьмо в кодировке windows-1251.\n\nС уважением,\nИТ-служба', 'plain', 'cp1251')
m['Subject'] = Header('Кодировка 1251', 'cp1251'); m['From'] = str(Header('ИТ-служба', 'cp1251')) + ' <it@rrcki.ru>'
m['To'] = 'test@rrcki.ru'; m['Date'] = formatdate(localtime=True)
append('INBOX', m.as_bytes())

# только HTML
m = MIMEText('<html><body><p>Привет, <b>мир</b>!</p><p>Вторая&nbsp;строка<br>третья</p><script>alert(1)</script></body></html>', 'html', 'utf-8')
m['Subject'] = Header('Только HTML', 'utf-8'); m['From'] = 'html@rrcki.ru'; m['To'] = 'test@rrcki.ru'
m['Date'] = formatdate(localtime=True)
append('INBOX', m.as_bytes())

# вложение с русским именем + текст
m = EmailMessage()
m['Subject'] = 'Отчёт с вложением'; m['From'] = 'Юшков Дмитрий <yushkov_do@nrcki.ru>'
m['To'] = 'test@rrcki.ru'; m['Cc'] = 'Коллега <colleague@nrcki.ru>'; m['Date'] = formatdate(localtime=True)
m.set_content('Во вложении отчёт.\n\nДмитрий')
m.add_attachment('Содержимое отчёта\n'.encode('utf-8'), maintype='text', subtype='plain', filename='Отчёт за октябрь.txt')
m.add_attachment(b'%PDF-1.4 fake', maintype='application', subtype='pdf', filename='../../evil.pdf')
append('INBOX', m.as_bytes(), '(\\Flagged)')

m = MIMEText('Письмо в папке с русским именем.', 'plain', 'utf-8')
m['Subject'] = Header('В проектной папке', 'utf-8'); m['From'] = 'pm@rrcki.ru'; m['To'] = 'test@rrcki.ru'
append(proj, m.as_bytes())
print(M.list())
M.logout()
