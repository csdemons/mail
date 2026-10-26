"""Создаёт в Radicale календарь и адресную книгу с тестовыми данными."""
import base64, datetime, os, ssl, urllib.request

BASE = 'https://localhost:5232'
USER, PASS = 'test@rrcki.ru', 'Secret-123'
ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
auth = 'Basic ' + base64.b64encode(f'{USER}:{PASS}'.encode()).decode()

def req(method, path, body=None, ctype='application/xml; charset=utf-8'):
    r = urllib.request.Request(BASE + path, data=body.encode() if body else None, method=method,
                               headers={'Authorization': auth, 'Content-Type': ctype})
    try:
        return urllib.request.urlopen(r, context=ctx).status
    except urllib.error.HTTPError as e:
        return e.code

home = f'/{USER}/'
print('home', req('PROPFIND', home, '<propfind xmlns="DAV:"><prop><resourcetype/></prop></propfind>'))
print('mkcal', req('MKCOL', home + 'work/', '''<?xml version="1.0"?><mkcol xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav" xmlns:A="http://apple.com/ns/ical/"><set><prop><resourcetype><collection/><C:calendar/></resourcetype><displayname>Рабочий</displayname><A:calendar-color>#e66b2e</A:calendar-color><C:supported-calendar-component-set><C:comp name="VEVENT"/></C:supported-calendar-component-set></prop></set></mkcol>'''))
print('mkbook', req('MKCOL', home + 'contacts/', '''<?xml version="1.0"?><mkcol xmlns="DAV:" xmlns:CR="urn:ietf:params:xml:ns:carddav"><set><prop><resourcetype><collection/><CR:addressbook/></resourcetype><displayname>Сотрудники</displayname></prop></set></mkcol>'''))

today = datetime.date.today()
monday = today - datetime.timedelta(days=today.weekday())
fmt = lambda d: d.strftime('%Y%m%d')
events = {
    'planerka': f'''BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\nUID:planerka\r\nSUMMARY:Планёрка УИТ\r\nLOCATION:Переговорная 214\r\nDTSTART;TZID=Europe/Moscow:{fmt(monday - datetime.timedelta(weeks=2))}T100000\r\nDTEND;TZID=Europe/Moscow:{fmt(monday - datetime.timedelta(weeks=2))}T103000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,TH\r\nORGANIZER;CN=Юшков Дмитрий:mailto:yushkov_do@nrcki.ru\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n''',
    'release': f'''BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\nUID:release\r\nSUMMARY:Обновление RuPost\r\nDESCRIPTION:Окно работ, почта недоступна\r\nDTSTART:{fmt(monday + datetime.timedelta(days=2))}T150000Z\r\nDTEND:{fmt(monday + datetime.timedelta(days=2))}T170000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n''',
    'trip': f'''BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\nUID:trip\r\nSUMMARY:Командировка\r\nDTSTART;VALUE=DATE:{fmt(monday + datetime.timedelta(days=3))}\r\nDTEND;VALUE=DATE:{fmt(monday + datetime.timedelta(days=5))}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n''',
}
for uid, ics in events.items():
    print('event', uid, req('PUT', home + f'work/{uid}.ics', ics, 'text/calendar; charset=utf-8'))

cards = {
    'ivanov': 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:ivanov\r\nFN:Иванов Иван Иванович\r\nN:Иванов;Иван;Иванович;;\r\nEMAIL;TYPE=work:ivanov@nrcki.ru\r\nTEL;TYPE=work:+7 495 000-00-01\r\nORG:НИЦ КИ;УИТ\r\nTITLE:Ведущий инженер\r\nEND:VCARD\r\n',
    'petrova': 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:petrova\r\nFN:Петрова Анна\r\nN:Петрова;Анна;;;\r\nEMAIL:petrova@nrcki.ru\r\nEMAIL:a.petrova@rrcki.ru\r\nORG:НИЦ КИ\r\nEND:VCARD\r\n',
    'noc': 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:noc\r\nFN:Дежурная смена NOC\r\nEMAIL:noc@nrcki.ru\r\nEND:VCARD\r\n',
}
for uid, vcf in cards.items():
    print('card', uid, req('PUT', home + f'contacts/{uid}.vcf', vcf, 'text/vcard; charset=utf-8'))
