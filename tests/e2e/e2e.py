import imaplib, ssl, subprocess, sys, time, os, glob, email
from email.header import decode_header, make_header
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import StaleElementReferenceException
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.common.keys import Keys

def clear(el):
    el.send_keys(Keys.CONTROL, 'a'); el.send_keys(Keys.BACKSPACE)

APP = os.environ.get('RRCKI_MAIL_BIN', os.path.join(os.path.dirname(__file__), '../../src-tauri/target/debug/rrcki-mail'))
OUT = sys.argv[1]
ok = lambda msg: print('PASS', msg, flush=True)

def session():
    opts = ArgOptions()
    opts.set_capability('browserName', 'wry')
    opts.set_capability('tauri:options', {'application': APP})
    d = webdriver.Remote('http://127.0.0.1:4444', options=opts)
    d.set_window_size(1400, 900)
    return d

def set_value(d, el, value):
    # Поля date/time и controlled-инпуты React: ставим значение через нативный сеттер.
    d.execute_script("""
      const el = arguments[0], v = arguments[1];
      const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
    """, el, value)

def field(d, label):
    for f in d.find_elements(By.CSS_SELECTOR, '.editor-modal .login-field'):
        if f.find_element(By.TAG_NAME, 'span').get_attribute('textContent').strip() == label:
            return f.find_element(By.CSS_SELECTOR, 'input, textarea, select')
    raise AssertionError(f'no field {label}')

def accept_confirm(d):
    # WebKitWebDriver по умолчанию сам принимает confirm(); если диалог всё же открыт — подтверждаем.
    try:
        W(d, 2).until(EC.alert_is_present()); d.switch_to.alert.accept()
    except Exception:
        pass

def dav_get(path):
    import base64, urllib.request
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    r = urllib.request.Request('https://localhost:5232' + path, method='PROPFIND', headers={
        'Authorization': 'Basic ' + base64.b64encode(b'test@rrcki.ru:Secret-123').decode(), 'Depth': '1'})
    return urllib.request.urlopen(r, context=ctx).read().decode()

def dav_delete(path):
    import base64, urllib.request
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    r = urllib.request.Request('https://localhost:5232' + path, method='DELETE', headers={
        'Authorization': 'Basic ' + base64.b64encode(b'test@rrcki.ru:Secret-123').decode()})
    return urllib.request.urlopen(r, context=ctx).status

def dav_put(path, body):
    import base64, urllib.request
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    r = urllib.request.Request('https://localhost:5232' + path, data=body.encode(), method='PUT', headers={
        'Authorization': 'Basic ' + base64.b64encode(b'test@rrcki.ru:Secret-123').decode(), 'Content-Type': 'text/vcard; charset=utf-8'})
    return urllib.request.urlopen(r, context=ctx).status

def nc_mock(method, path):
    import urllib.request
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    r = urllib.request.Request('https://localhost:8443' + path, method=method, data=b'' if method == 'POST' else None)
    import json as _j
    return _j.loads(urllib.request.urlopen(r, context=ctx).read() or b'{}')

def W(d, t):
    # Список перерисовывается после каждого действия — устаревшие элементы не ошибка.
    return WebDriverWait(d, t, ignored_exceptions=(StaleElementReferenceException,))

def wait(d, css, t=20):
    return W(d, t).until(EC.presence_of_element_located((By.CSS_SELECTOR, css)))

def server_flags(uid):
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('test@rrcki.ru', 'Secret-123')
    M.select('INBOX'); _, data = M.uid('FETCH', str(uid), '(FLAGS)'); M.logout(); return data[0].decode()

def login(d, password):
    wait(d, '.login-card')
    clear(d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]'))
    d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').send_keys('test@rrcki.ru')
    d.find_element(By.CSS_SELECTOR, 'input[type=password]').send_keys(password)
    toggles = d.find_elements(By.CSS_SELECTOR, '.login-server-toggle')
    if toggles: toggles[0].click()
    host, port, smtp_host, smtp_port = d.find_elements(By.CSS_SELECTOR, '.login-server input')
    clear(host); host.send_keys('localhost'); clear(port); port.send_keys('10993')
    clear(smtp_host); smtp_host.send_keys('localhost'); clear(smtp_port); smtp_port.send_keys('465')
    dav = [i for i in d.find_elements(By.CSS_SELECTOR, '.login-field input') if 'автоопределение' in (i.get_attribute('placeholder') or '')][0]
    clear(dav); dav.send_keys('https://localhost:5232/')
    d.find_element(By.CSS_SELECTOR, '.login-submit').click()

d = session()
try:
    # 1. неверный пароль
    login(d, 'wrong')
    err = wait(d, '.login-error').text
    assert 'Неверный логин или пароль' in err, err; ok(f'wrong password -> «{err}»')
    ver = __import__('json').load(open(os.path.join(os.path.dirname(__file__), '../../src-tauri/tauri.conf.json')))['version']
    W(d, 10).until(lambda d: f'RRCKI Mail {ver}' in d.find_element(By.CSS_SELECTOR, '.login-version').get_attribute('textContent'))
    ok(f'login screen shows version {ver}')
    d.save_screenshot(f'{OUT}/01-login-error.png')

    # 2. вход
    clear(d.find_element(By.CSS_SELECTOR, 'input[type=password]'))
    login(d, 'Secret-123')
    wait(d, '.message-row')
    time.sleep(1)
    rows = d.find_elements(By.CSS_SELECTOR, '.message-row')
    assert len(rows) == 50, len(rows); ok('first page: 50 rows')

    # Обновление: уведомление о новой версии, «Что нового», загрузка с проверкой SHA-256.
    card = W(d, 40).until(lambda d: d.find_element(By.CSS_SELECTOR, '.update-card'))
    W(d, 10).until(lambda d: 'Доступна версия 9.9.9' in d.find_element(By.CSS_SELECTOR, '.update-card').get_attribute('textContent'))
    [b for b in card.find_elements(By.TAG_NAME, 'button') if 'Что нового' in b.text][0].click()
    assert 'Проверка обновлений' in wait(d, '.update-notes').get_attribute('textContent'); ok('update available: 9.9.9 with release notes')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.update-card button') if 'Обновить' in b.text][0].click()
    W(d, 30).until(lambda d: 'скачана' in d.find_element(By.CSS_SELECTOR, '.update-card').get_attribute('textContent'))
    cmd = d.find_element(By.CSS_SELECTOR, '.update-cmd').get_attribute('textContent')
    path = cmd.split(': ', 1)[1]
    assert open(path, 'rb').read() == b'fake package for update test\n' * 4000, path
    ok('update downloaded, SHA-256 verified, manual install hint shown')
    d.find_element(By.CSS_SELECTOR, '.update-card button[aria-label="Закрыть"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.update-card'))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    about = W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, '.about-block'))
    W(d, 10).until(lambda d: f'RRCKI Mail {ver}' in d.find_element(By.CSS_SELECTOR, '.about-block').get_attribute('textContent'))
    t = about.get_attribute('textContent')
    assert 'НИЦ «Курчатовский институт»' in t and 'Юшков Дмитрий Олегович' in t, t; ok('settings: version and «О программе»')
    d.save_screenshot(f'{OUT}/12-about.png')
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
    folders = [e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '.folder-item')]
    print('   folders:', folders)
    assert any('Входящие' in f for f in folders) and any('Проекты' in f for f in folders), folders
    assert not any('2026' in f for f in folders), 'subfolders must be collapsed by default'
    ok('folder tree, subfolders collapsed by default')
    d.save_screenshot(f'{OUT}/02-inbox.png')

    # 3. догрузка
    d.find_element(By.CSS_SELECTOR, '.load-more').click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.message-row')) == 100); ok('load more: 100 rows')

    # 4. письмо с вложениями
    flags_before = server_flags(123)
    row = [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Отчёт с вложением' in r.get_attribute('textContent')][0]
    assert 'unread' in row.get_attribute('class'); row.click()
    wait(d, '.attachment-card')
    names = [e.find_element(By.TAG_NAME, 'strong').get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '.attachment-card')]
    assert names == ['Отчёт за октябрь.txt', 'evil.pdf'], names; ok(f'attachments {names}')
    body = d.find_element(By.CSS_SELECTOR, '.message-body').text
    assert 'Во вложении отчёт' in body and '\n' in body, repr(body); ok('body with line breaks')
    time.sleep(1.5)
    flags_after = server_flags(123)
    assert '\\Seen' not in flags_before and '\\Seen' in flags_after, (flags_before, flags_after); ok('\\Seen set on server')
    d.save_screenshot(f'{OUT}/03-message.png')

    # 5. HTML-письмо → текст
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Только HTML' in r.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: 'Привет, мир!' in d.find_element(By.CSS_SELECTOR, '.message-body').text)
    assert 'alert' not in d.find_element(By.CSS_SELECTOR, '.message-body').text; ok('HTML rendered as text, script dropped')

    # 6. флажок
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Пометить"]').click(); time.sleep(1.5)
    assert '\\Flagged' in server_flags(122); ok('\\Flagged set on server')

    # 7. кириллическая папка: раскрыть «Проекты», открыть «2026»
    [f for f in d.find_elements(By.CSS_SELECTOR, '.folder-node') if 'Проекты' in f.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: any('2026' in f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item')))
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if '2026' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.message-row')) == 1 and 'В проектной папке' in d.find_element(By.CSS_SELECTOR, '.message-row').get_attribute('textContent'))
    ok('Cyrillic folder opened')
    d.save_screenshot(f'{OUT}/04-folder.png')

    OUTBOX = '/srv/imaptest/outbox'
    def outbox():
        return sorted(glob.glob(f'{OUTBOX}/*.eml'))
    def subject_of(path):
        raw = open(path, 'rb').read()
        return str(make_header(decode_header(email.message_from_bytes(raw)['Subject']))), raw

    # 8. контакты (CardDAV)
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Контакты']]").click()
    W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.contact-row')) == 3)
    names = [r.find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.contact-row')]
    assert names == ['Дежурная смена NOC', 'Иванов Иван Иванович', 'Петрова Анна'], names; ok(f'contacts {names}')
    [r for r in d.find_elements(By.CSS_SELECTOR, '.contact-row') if 'Иванов' in r.get_attribute('textContent')][0].click()
    card = wait(d, '.contact-card').get_attribute('textContent')
    assert 'ivanov@nrcki.ru' in card and 'Ведущий инженер' in card and 'НИЦ КИ, УИТ' in card, card; ok('contact card')
    assert dav_put('/test@rrcki.ru/contacts/kuznetsova.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:kuznetsova\r\nFN:Кузнецова Мария\r\nEMAIL:kuznetsova@nrcki.ru\r\nEND:VCARD\r\n') in (200, 201, 204)
    search = d.find_element(By.CSS_SELECTOR, '.contacts-view .search-box input') if d.find_elements(By.CSS_SELECTOR, '.contacts-view .search-box input') else d.find_element(By.CSS_SELECTOR, 'input[placeholder="Имя, e-mail, телефон, отдел"]')
    search.send_keys('кузнец')
    W(d, 10).until(lambda d: any('Кузнецова Мария' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.contact-row')))
    ok('contacts search finds entries on the server')
    clear(search); dav_delete('/test@rrcki.ru/contacts/kuznetsova.vcf')
    d.save_screenshot(f'{OUT}/06-contacts.png')

    # 9. письмо из карточки контакта (SMTP + копия в «Отправленные»)
    before = len(outbox())
    [b for b in d.find_elements(By.CSS_SELECTOR, '.contact-card button') if 'Написать' in b.text][0].click()
    wait(d, '.compose-modal')
    to = d.find_element(By.CSS_SELECTOR, '.recipient-field input').get_attribute('value')
    assert 'ivanov@nrcki.ru' in to, to
    subj = d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'); subj.send_keys('Проверка отправки из RRCKI Mail')
    body = d.find_element(By.CSS_SELECTOR, '.compose-body')
    body.send_keys('Добрый день!'); body.send_keys(Keys.ENTER); body.send_keys('Это тест.')
    # Оформление: выделить всё, шрифт Georgia, жирный, крупнее.
    body.send_keys(Keys.CONTROL, 'a')
    from selenium.webdriver.support.ui import Select
    Select(d.find_element(By.CSS_SELECTOR, '.rt-font')).select_by_visible_text('Georgia')
    d.find_element(By.CSS_SELECTOR, '.rt-btn[aria-label^="Жирный"]').click()
    Select(d.find_element(By.CSS_SELECTOR, '.rt-size')).select_by_visible_text('Очень крупный')
    W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.rt-btn[aria-label^="Жирный"]').get_attribute('aria-pressed') == 'true')
    d.save_screenshot(f'{OUT}/05-compose-format.png')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 30).until(lambda d: 'Письмо отправлено' in (d.find_elements(By.CSS_SELECTOR, '.toast') or [None])[0].get_attribute('textContent') if d.find_elements(By.CSS_SELECTOR, '.toast') else False)
    W(d, 10).until(lambda d: len(outbox()) == before + 1)
    s1, raw1 = subject_of(outbox()[-1])
    assert s1 == 'Проверка отправки из RRCKI Mail' and b'ivanov@nrcki.ru' in raw1, s1; ok('sent via SMTP, Cyrillic subject intact')
    m1 = email.message_from_bytes(raw1)
    parts = {p.get_content_type(): p.get_payload(decode=True).decode(p.get_content_charset() or 'utf-8') for p in m1.walk() if not p.is_multipart()}
    assert m1.get_content_type() == 'multipart/alternative' and set(parts) == {'text/plain', 'text/html'}, (m1.get_content_type(), list(parts))
    assert parts['text/plain'].replace('\r\n', '\n').strip() == 'Добрый день!\nЭто тест.', repr(parts['text/plain'])
    h = parts['text/html']
    assert 'Georgia' in h and ('bold' in h or '<b>' in h) and ('x-large' in h or 'size="5"' in h) and 'Добрый день!' in h, h
    ok('formatted mail: multipart/alternative, Georgia + bold + size in HTML, clean plain text')
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('test@rrcki.ru', 'Secret-123')
    typ, data = M.select('Sent'); M.logout()
    assert int(data[0]) >= 1, data; ok(f'copy saved to Sent ({int(data[0])})')

    # 10. ответ: In-Reply-To + подсказка адреса из контактов
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Входящие' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: any('Отчёт с вложением' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Отчёт с вложением' in r.get_attribute('textContent')][0].click()
    wait(d, '.attachment-card')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.reader-actions button') if 'Ответить' in b.text][0].click()
    wait(d, '.compose-modal')
    fields = d.find_elements(By.CSS_SELECTOR, '.recipient-field input')
    assert 'yushkov_do@nrcki.ru' in fields[0].get_attribute('value'); 
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'Re: Отчёт с вложением'
    d.find_element(By.CSS_SELECTOR, '.add-cc').click()
    # Запись «как в корпоративной книге»: появилась на сервере после загрузки списка,
    # клиент о ней не знает — найти её можно только поиском на сервере.
    assert dav_put('/test@rrcki.ru/contacts/sidorov.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:sidorov\r\nFN:Сидоров Олег\r\nN:Сидоров;Олег;;;\r\nEMAIL:sidorov@nrcki.ru\r\nORG:НИЦ КИ\\, ОИБ\r\nTITLE:Аналитик\r\nEND:VCARD\r\n') in (200, 201, 204)
    cc = d.find_elements(By.CSS_SELECTOR, '.recipient-field input')[1]; cc.send_keys('Сидор')
    sug = W(d, 10).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.suggestions button') if 'sidorov@nrcki.ru' in b.get_attribute('textContent')])
    assert 'Аналитик' in sug[0].get_attribute('textContent'); ok('recipient found by server-side address book search (GAL-like)')
    clear(cc); dav_delete('/test@rrcki.ru/contacts/sidorov.vcf')
    cc.send_keys('Петр')
    sug = W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.suggestions button'))
    labels = [b.get_attribute('textContent') for b in sug]
    assert any('petrova@nrcki.ru' in l for l in labels), labels
    sug[0].click(); time.sleep(0.3)
    assert 'Петрова Анна <petrova@nrcki.ru>' in cc.get_attribute('value'), cc.get_attribute('value'); ok('address autocomplete from contacts')
    d.save_screenshot(f'{OUT}/07-reply.png')
    before = len(outbox())
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 30).until(lambda d: len(outbox()) == before + 1)
    s2, raw2 = subject_of(outbox()[-1])
    assert s2 == 'Re: Отчёт с вложением' and b'petrova@nrcki.ru' in raw2, (s2, raw2[:400])
    html2 = [p for p in email.message_from_bytes(raw2).walk() if p.get_content_type() == 'text/html'][0].get_payload(decode=True).decode()
    assert '<blockquote' in html2, html2; ok('reply quote sent as blockquote')
    assert b'In-Reply-To: <report-oct@nrcki.ru>' in raw2 and b'References: <thread-root@nrcki.ru> <report-oct@nrcki.ru>' in raw2, raw2[:700]
    ok('reply has In-Reply-To, References and Cc')

    # 11. календарь (CalDAV)
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Календарь']]").click()
    W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.event-block')) >= 3)
    blocks = [b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')]
    chips = [b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-chip')]
    assert sum('Планёрка УИТ' in b for b in blocks) == 2 and any('Обновление RuPost' in b for b in blocks), blocks
    assert chips.count('Командировка') == 2, chips; ok(f'week view: {len(blocks)} timed events, all-day spans 2 days')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.event-block') if 'Планёрка' in b.get_attribute('textContent')][0].click()
    det = wait(d, '.event-details').get_attribute('textContent')
    assert 'Повторяющееся событие' in det and 'Переговорная 214' in det and 'Юшков Дмитрий' in det, det; ok('event details (recurring, location, organizer)')
    d.save_screenshot(f'{OUT}/08-calendar.png')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Следующая неделя"]').click()
    W(d, 20).until(lambda d: sum('Планёрка' in b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')) == 2 and not d.find_elements(By.CSS_SELECTOR, '.event-chip'))
    ok('next week: recurrence continues, no all-day trip')
    d.find_element(By.XPATH, "//button[text()='Сегодня']").click()
    W(d, 20).until(lambda d: any('Обновление RuPost' in b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')))

    # 12. создание, изменение и удаление события
    import datetime
    monday = datetime.date.today() - datetime.timedelta(days=datetime.date.today().weekday())
    wed = (monday + datetime.timedelta(days=2)).isoformat()
    d.find_element(By.CSS_SELECTOR, '.create-event').click()
    wait(d, '.editor-modal')
    set_value(d, field(d, 'Название'), 'Тестовое совещание')
    set_value(d, field(d, 'Начало'), wed); set_value(d, field(d, 'Окончание'), wed)
    times = d.find_elements(By.CSS_SELECTOR, '.editor-modal input[type=time]')
    set_value(d, times[0], '11:00'); set_value(d, times[1], '12:30')
    set_value(d, field(d, 'Место'), 'Каб. 101')
    d.find_element(By.CSS_SELECTOR, '.editor-modal button[type=submit]').click()
    W(d, 20).until(lambda d: any('Тестовое совещание' in b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')))
    assert 'SUMMARY:Тестовое совещание' in ''.join(open(f).read() for f in glob.glob('/srv/imaptest/dav/collections/collection-root/test@rrcki.ru/work/*.ics'))
    ok('event created (UI + server)')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.event-block') if 'Тестовое совещание' in b.get_attribute('textContent')][0].click()
    [b for b in wait(d, '.event-details').find_elements(By.TAG_NAME, 'button') if 'Изменить' in b.text][0].click()
    wait(d, '.editor-modal')
    set_value(d, field(d, 'Название'), 'Совещание перенесено')
    times = d.find_elements(By.CSS_SELECTOR, '.editor-modal input[type=time]')
    set_value(d, times[0], '15:00'); set_value(d, times[1], '16:00')
    d.find_element(By.CSS_SELECTOR, '.editor-modal button[type=submit]').click()
    W(d, 20).until(lambda d: any('Совещание перенесено' in b.get_attribute('textContent') and '15:00' in b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')))
    ok('event edited: title and time')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.event-block') if 'Совещание перенесено' in b.get_attribute('textContent')][0].click()
    [b for b in wait(d, '.event-details').find_elements(By.TAG_NAME, 'button') if 'Удалить' in b.text][0].click()
    accept_confirm(d)
    W(d, 20).until(lambda d: not any('Совещание перенесено' in b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')))
    assert 'Совещание перенесено' not in ''.join(open(f).read() for f in glob.glob('/srv/imaptest/dav/collections/collection-root/test@rrcki.ru/work/*.ics'))
    ok('event deleted (UI + server)')

    # 13. повторяющееся событие: время заблокировано, место меняется у всей серии
    [b for b in d.find_elements(By.CSS_SELECTOR, '.event-block') if 'Планёрка' in b.get_attribute('textContent')][0].click()
    [b for b in wait(d, '.event-details').find_elements(By.TAG_NAME, 'button') if 'Изменить' in b.text][0].click()
    wait(d, '.editor-modal')
    assert all(not i.is_enabled() for i in d.find_elements(By.CSS_SELECTOR, '.editor-modal input[type=time], .editor-modal input[type=date]'))
    set_value(d, field(d, 'Место'), 'Переговорная 305')
    d.find_element(By.CSS_SELECTOR, '.editor-modal button[type=submit]').click()
    W(d, 20).until(lambda d: sum('Переговорная 305' in (b.get_attribute('title') or '') + b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')) >= 0 and not d.find_elements(By.CSS_SELECTOR, '.editor-modal'))
    ics = open('/srv/imaptest/dav/collections/collection-root/test@rrcki.ru/work/planerka.ics').read()
    assert 'LOCATION:Переговорная 305' in ics and 'RRULE:FREQ=WEEKLY;BYDAY=MO,TH' in ics and 'ORGANIZER' in ics and 'SEQUENCE:1' in ics, ics
    ok('recurring series: time locked, location updated, RRULE/ORGANIZER kept')
    d.save_screenshot(f'{OUT}/09-calendar-edit.png')

    # 14. контакты: создание, изменение, удаление
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Контакты']]").click()
    W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.contact-row')) == 3)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Новый контакт"]').click()
    wait(d, '.editor-modal')
    set_value(d, field(d, 'Фамилия Имя Отчество'), 'Сидоров Пётр Ильич')
    set_value(d, field(d, 'E-mail (по одному в строке)'), 'sidorov@nrcki.ru')
    set_value(d, field(d, 'Организация, отдел'), 'НИЦ КИ, УИТ')
    d.find_element(By.CSS_SELECTOR, '.editor-modal button[type=submit]').click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.contact-row')) == 4)
    ok('contact created')
    [r for r in d.find_elements(By.CSS_SELECTOR, '.contact-row') if 'Сидоров' in r.get_attribute('textContent')][0].click()
    [b for b in wait(d, '.contact-card').find_elements(By.TAG_NAME, 'button') if 'Изменить' in b.text][0].click()
    wait(d, '.editor-modal')
    set_value(d, field(d, 'Телефоны'), '+7 495 000-00-02')
    set_value(d, field(d, 'Должность'), 'Инженер')
    d.find_element(By.CSS_SELECTOR, '.editor-modal button[type=submit]').click()
    W(d, 20).until(lambda d: '+7 495 000-00-02' in d.find_element(By.CSS_SELECTOR, '.contact-card').get_attribute('textContent'))
    vcf = ''.join(open(f).read() for f in glob.glob('/srv/imaptest/dav/collections/collection-root/test@rrcki.ru/contacts/*.vcf'))
    assert 'N:Сидоров;Пётр;Ильич;;' in vcf and 'TITLE:Инженер' in vcf and 'ORG:НИЦ КИ;УИТ' in vcf, vcf
    ok('contact edited (server vCard N/ORG/TITLE correct)')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.contact-card button') if 'Удалить' in b.text][0].click()
    accept_confirm(d)
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.contact-row')) == 3)
    ok('contact deleted')

    # 17. Nextcloud: подключение через Login Flow v2
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Задачи']]").click()
    card = wait(d, '.nc-connect.card')
    set_value(d, card.find_element(By.CSS_SELECTOR, 'input'), 'https://localhost:8443/')
    d.find_element(By.CSS_SELECTOR, '.nc-start').click()
    W(d, 40).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item')) == 2)
    assert nc_mock('GET', '/test/state')['polls'] >= 3; ok('Nextcloud connected via login flow (browser approval simulated)')

    # 18. Deck
    boards = [b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item')]
    assert boards == ['Объявления', 'УИТ — задачи'], boards
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item') if 'УИТ' in b.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: [c.find_element(By.TAG_NAME, 'strong').text for c in d.find_elements(By.CSS_SELECTOR, '.deck-column')] == ['К выполнению', 'В работе', 'Готово'])
    first = d.find_elements(By.CSS_SELECTOR, '.deck-column')[0]
    txt = first.get_attribute('textContent')
    assert 'Обновить RuPost' in txt and 'Срочно' in txt, txt; ok('board columns, card labels')
    first.find_element(By.CSS_SELECTOR, '.deck-add-button').click()
    inp = W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.deck-add input'))
    inp.send_keys('Проверить резервные копии'); inp.send_keys(Keys.ENTER)
    W(d, 20).until(lambda d: 'Проверить резервные копии' in d.find_elements(By.CSS_SELECTOR, '.deck-column')[0].get_attribute('textContent'))
    new = [c for c in nc_mock('GET', '/test/state')['cards'] if c['title'] == 'Проверить резервные копии'][0]
    assert new['stackId'] == 10; ok('card created')
    # Перетаскивание мышью (pointer-события) в «В работе».
    from selenium.webdriver.common.action_chains import ActionChains
    src = [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if 'Проверить резервные копии' in c.get_attribute('textContent')][0]
    dst = d.find_elements(By.CSS_SELECTOR, '.deck-column')[1]
    ac = ActionChains(d).move_to_element(src).click_and_hold()
    for i in range(1, 6):
        ac = ac.move_to_element_with_offset(dst, 0, 0) if i == 5 else ac.move_by_offset(30 * i, 4)
    ac.pause(0.2).release().perform()
    W(d, 20).until(lambda d: [c for c in nc_mock('GET', '/test/state')['cards'] if c['id'] == new['id']][0]['stackId'] == 11)
    W(d, 20).until(lambda d: 'Проверить резервные копии' in d.find_elements(By.CSS_SELECTOR, '.deck-column')[1].get_attribute('textContent'))
    assert not d.find_elements(By.CSS_SELECTOR, '.deck-panel'), 'drag must not open the card'
    ok('card dragged to «В работе» with the mouse')
    [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if 'Проверить резервные копии' in c.get_attribute('textContent')][0].click()
    panel = wait(d, '.deck-panel')
    set_value(d, panel.find_element(By.CSS_SELECTOR, 'textarea'), 'Veeam + NAS')
    set_value(d, panel.find_element(By.CSS_SELECTOR, 'input[type=date]'), '2026-11-05')
    panel.find_element(By.CSS_SELECTOR, '.deck-done input').click()
    [b for b in panel.find_elements(By.TAG_NAME, 'button') if 'Сохранить' in b.text][0].click()
    W(d, 20).until(lambda d: [c for c in nc_mock('GET', '/test/state')['cards'] if c['id'] == new['id']][0]['description'] == 'Veeam + NAS')
    saved = [c for c in nc_mock('GET', '/test/state')['cards'] if c['id'] == new['id']][0]
    assert saved['done'] and saved['duedate'].startswith('2026-11-05'), saved; ok('card edited: description, due date, done')
    from selenium.webdriver.support.ui import Select
    Select(wait(d, '.deck-panel select')).select_by_visible_text('Готово')
    W(d, 20).until(lambda d: [c for c in nc_mock('GET', '/test/state')['cards'] if c['id'] == new['id']][0]['stackId'] == 12)
    W(d, 20).until(lambda d: 'Проверить резервные копии' in d.find_elements(By.CSS_SELECTOR, '.deck-column')[2].get_attribute('textContent'))
    ok('card moved to «Готово»')
    d.save_screenshot(f'{OUT}/10-deck.png')
    [b for b in wait(d, '.deck-panel').find_elements(By.TAG_NAME, 'button') if 'Удалить' in b.text][0].click()
    accept_confirm(d)
    W(d, 20).until(lambda d: not any(c['id'] == new['id'] for c in nc_mock('GET', '/test/state')['cards']))
    ok('card deleted')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item') if 'Объявления' in b.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: 'Новости' in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('textContent'))
    assert not d.find_elements(By.CSS_SELECTOR, '.deck-add-button') and 'только чтение' in d.find_element(By.CSS_SELECTOR, '.deck-main').get_attribute('textContent')
    ok('read-only board: no editing controls')

    # 19. Talk
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.room-row')) == 2)
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.talk-msg')) == 2)
    msgs = [m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg')]
    assert '@Тестовый Пользователь' in msgs[0] and 'План работ.xlsx' in msgs[1], msgs
    assert [f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, '.talk-msg .talk-file span')] == ['План работ.xlsx']
    assert not any('reaction' in m for m in msgs); ok('history: mentions and files rendered, reaction events hidden')
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.rail-badge')); ok('chat badge cleared after reading')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('Добрый день, принято'); box.send_keys(Keys.ENTER)
    W(d, 20).until(lambda d: any('Добрый день, принято' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own')))
    ok('message sent, shown as own')
    t0 = time.time()
    nc_mock('POST', '/test/inject?token=abc123&text=' + __import__('urllib.parse').parse.quote('Срочно: проверьте почту'))
    W(d, 15).until(lambda d: any('Срочно: проверьте почту' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg:not(.own)')))
    ok(f'incoming message via long-poll in {time.time() - t0:.1f}s')
    target = [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if 'обновление в среду' in m.get_attribute('textContent')][0]
    d.execute_script("arguments[0].click()", target.find_element(By.CSS_SELECTOR, '.talk-actions button[aria-label="Ответить"]'))
    wait(d, '.talk-quote.replying')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'); box.send_keys('Буду в среду'); box.send_keys(Keys.ENTER)
    W(d, 20).until(lambda d: any('Буду в среду' in m.get_attribute('textContent') and 'обновление в среду' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own')))
    last = nc_mock('GET', '/test/state')['messages']['abc123'][-1]
    assert last['message'] == 'Буду в среду' and last['parent']['id'] == 1, last; ok('reply with quote')
    # Файл: скрепка → (диалог подменён RRCKI_E2E_PICK) → плашка → отправка с подписью.
    d.find_element(By.CSS_SELECTOR, '.talk-attach').click()
    W(d, 10).until(lambda d: 'отчёт-e2e.txt' in d.find_element(By.CSS_SELECTOR, '.talk-pending').get_attribute('textContent'))
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'); box.send_keys('Логи во вложении'); box.send_keys(Keys.ENTER)
    W(d, 20).until(lambda d: any('Логи во вложении' in m.get_attribute('textContent') and 'отчёт-e2e.txt' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own')))
    st = nc_mock('GET', '/test/state')
    assert st['files'].get('/Talk/отчёт-e2e.txt') == 'test file for talk\n', st['files']
    assert not d.find_elements(By.CSS_SELECTOR, '.talk-pending'); ok('file uploaded to /Talk and shared with caption')
    # Второй раз то же имя → «отчёт-e2e (1).txt», без перезаписи.
    d.find_element(By.CSS_SELECTOR, '.talk-attach').click()
    wait(d, '.talk-pending')
    d.find_element(By.CSS_SELECTOR, '.talk-compose button[type=submit]').click()
    W(d, 20).until(lambda d: '/Talk/отчёт-e2e (1).txt' in nc_mock('GET', '/test/state')['files'])
    ok('same name not overwritten: «отчёт-e2e (1).txt»')

    def talk_msg(text):
        return [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if text in m.get_attribute('textContent')][0]
    def action(msg, label):
        d.execute_script("arguments[0].click()", msg.find_element(By.CSS_SELECTOR, f'.talk-actions button[aria-label="{label}"]'))
    def msg_state(mid):
        return next(m for m in nc_mock('GET', '/test/state')['messages']['abc123'] if m['id'] == mid)

    # Реакция: поставить 👍 и снять.
    action(talk_msg('обновление в среду'), 'Реакция')
    d.execute_script("arguments[0].click()", wait(d, '.talk-react-picker button[aria-label="Реакция 👍"]'))
    W(d, 10).until(lambda d: '👍' in msg_state(1).get('reactionsSelf', []))
    W(d, 10).until(lambda d: talk_msg('обновление в среду').find_elements(By.CSS_SELECTOR, '.talk-reaction.own'))
    chip = talk_msg('обновление в среду').find_element(By.CSS_SELECTOR, '.talk-reaction.own')
    assert '👍' in chip.text and '1' in chip.text, chip.text
    chip.click()
    W(d, 10).until(lambda d: not msg_state(1).get('reactionsSelf') and not talk_msg('обновление в среду').find_elements(By.CSS_SELECTOR, '.talk-reaction'))
    ok('reaction added and removed')

    # Правка своего сообщения: обновление приходит через long-poll (служебное сообщение с parent).
    action(talk_msg('Добрый день, принято'), 'Изменить')
    wait(d, '.talk-quote.editing')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    assert box.get_attribute('value') == 'Добрый день, принято', box.get_attribute('value')
    box.send_keys(' — исправлено'); box.send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: 'исправлено' in talk_msg('Добрый день, принято').get_attribute('textContent') and 'изменено' in talk_msg('Добрый день, принято').get_attribute('textContent'))
    ok('own message edited, update arrived via long-poll')

    # Удаление своего сообщения.
    action(talk_msg('Буду в среду'), 'Удалить'); accept_confirm(d)
    W(d, 15).until(lambda d: not [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if 'Буду в среду' in m.get_attribute('textContent')]
                   and d.find_elements(By.CSS_SELECTOR, '.talk-msg.deleted'))
    ok('own message deleted')

    # Упоминание: «@Петр» → подсказка → «@Петрова Анна», в Talk уходит @"petrova".
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('@Петр')
    W(d, 10).until(lambda d: any('Петрова Анна' in b.text for b in d.find_elements(By.CSS_SELECTOR, '.talk-mentions button')))
    box.send_keys(Keys.ENTER)
    W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == '@Петрова Анна ')
    box.send_keys('посмотри **срочно** https://nrcki.ru/doc'); box.send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['messages']['abc123'][-1].get('raw') == '@"petrova" посмотри **срочно** https://nrcki.ru/doc')
    mm = W(d, 15).until(lambda d: talk_msg('посмотри'))
    W(d, 10).until(lambda d: talk_msg('посмотри').find_elements(By.CSS_SELECTOR, '.talk-mention'))
    assert talk_msg('посмотри').find_element(By.CSS_SELECTOR, 'b').text == 'срочно'
    assert talk_msg('посмотри').find_element(By.CSS_SELECTOR, 'a').get_attribute('href') == 'https://nrcki.ru/doc'
    ok('mention autocomplete (@"id"), bold and link rendered')

    # Эмодзи из панели.
    d.find_element(By.CSS_SELECTOR, '.talk-emoji').click()
    d.find_element(By.CSS_SELECTOR, '.emoji-grid button[aria-label="😀"]').click()
    W(d, 5).until(lambda d: '😀' in d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value'))
    d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['messages']['abc123'][-1].get('raw') == '😀')
    ok('emoji picker inserts and sends')
    d.save_screenshot(f'{OUT}/11-chat.png')

    # Карточка Deck из сообщения.
    action(talk_msg('обновление в среду'), 'Карточка Deck')
    modal = wait(d, '.talk-card-modal')
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.talk-card-modal select option')) >= 2)
    [b for b in modal.find_elements(By.TAG_NAME, 'button') if 'Создать карточку' in b.text][0].click()
    W(d, 15).until(lambda d: any(c['title'].startswith('Коллеги') and 'call/abc123#message_1' in c['description'] for c in nc_mock('GET', '/test/state')['cards']))
    ok('Deck card created from message (text, author, link)')

    # Новая личная беседа.
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Новая беседа"]').click()
    wait(d, '.talk-new-modal input').send_keys('Сидор')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-user-list .message-row'))[0].click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-new-modal button') if b.text == 'Написать'][0].click()
    W(d, 15).until(lambda d: 'Сидоров Олег' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').text)
    ok('new 1:1 conversation created and opened')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 15. перезапуск → автовход из keyring
d = session()
try:
    wait(d, '.message-row', 30); ok('auto-login from keyring after restart')
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.room-row')) == 3 and not d.find_elements(By.CSS_SELECTOR, '.nc-connect.card'))
    ok('Nextcloud still connected after restart (app password from keyring)')
    d.find_element(By.CSS_SELECTOR, '.account-switch').click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.account-menu button') if 'Выйти' in b.text][0].click()
    wait(d, '.login-card'); ok('logout -> login screen')
finally:
    d.quit()

# 16. после выхода пароль забыт
d = session()
try:
    wait(d, '.login-card', 30)
    assert d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').get_attribute('value') == 'test@rrcki.ru'
    ok('after logout: login screen, username prefilled, password not remembered')
finally:
    d.quit()
print('ALL E2E PASSED')
