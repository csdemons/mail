import json, imaplib, ssl, subprocess, sys, time, os, glob, email
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
    W(d, 10).until(lambda d: f'Atom Mail {ver}' in d.find_element(By.CSS_SELECTOR, '.login-version').get_attribute('textContent'))
    ok(f'login screen shows version {ver}')
    d.save_screenshot(f'{OUT}/01-login-error.png')

    # 2. вход
    clear(d.find_element(By.CSS_SELECTOR, 'input[type=password]'))
    login(d, 'Secret-123')
    wait(d, '.message-row')
    # Отмена отправки проверяется отдельно (14d); в остальных проверках письма уходят сразу.
    d.execute_script("localStorage.setItem('rrcki-mail.send-prefs', JSON.stringify({undoSecs: 0}))")
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
    W(d, 10).until(lambda d: f'Atom Mail {ver}' in d.find_element(By.CSS_SELECTOR, '.about-block').get_attribute('textContent'))
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
    frame = W(d, 20).until(lambda d: d.find_element(By.CSS_SELECTOR, 'iframe.message-html'))
    d.switch_to.frame(frame)
    W(d, 10).until(lambda d: 'Привет, мир!' in d.find_element(By.TAG_NAME, 'body').text)
    assert d.find_elements(By.TAG_NAME, 'b') and not d.find_elements(By.TAG_NAME, 'script')
    d.switch_to.default_content()
    ok('HTML mail rendered as HTML in sandboxed frame, script dropped')

    # 6. флажок
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Флажок"]').click(); time.sleep(1.5)
    assert '\\Flagged' in server_flags(122); ok('\\Flagged set on server')

    # 7. кириллическая папка: раскрыть «Проекты», открыть «2026»
    [f for f in d.find_elements(By.CSS_SELECTOR, '.folder-node') if 'Проекты' in f.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: any('2026' in f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item')))
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if '2026' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.message-row')) == 1 and 'В проектной папке' in d.find_element(By.CSS_SELECTOR, '.message-row').get_attribute('textContent'))
    ok('Cyrillic folder opened')

    # Тёмная тема: непрочитанные выделены, окно письма отличимо от фона, адрес в «Кому» читается.
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Входящие' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.message-row.unread'))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Тема"]').click()
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.app-shell.theme-dark'))
    bg = d.execute_script("return getComputedStyle(document.querySelector('.message-row.unread')).backgroundColor")
    assert bg in ('rgb(34, 50, 75)', 'rgba(34, 50, 75, 1)'), bg
    time.sleep(0.5); d.save_screenshot(f'{OUT}/14-dark-inbox.png')
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    to = wait(d, '.compose-modal .recipient-field input'); to.send_keys('Иванов Иван <ivanov@nrcki.ru>')
    assert d.execute_script("return getComputedStyle(arguments[0]).color", to) == 'rgb(244, 247, 252)'
    sel = d.find_element(By.CSS_SELECTOR, '.compose-modal .rt-font')
    assert d.execute_script("return getComputedStyle(arguments[0]).backgroundColor", sel) == 'rgb(28, 35, 48)'
    d.save_screenshot(f'{OUT}/15-dark-compose.png')
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Тема"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.app-shell.theme-dark'))
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    to = wait(d, '.compose-modal .recipient-field input')
    assert d.execute_script("return getComputedStyle(arguments[0]).color", to) == 'rgb(15, 23, 42)'
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    ok('dark theme: unread rows highlighted, compose window and recipients readable; light: recipients near-black')
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
    ev = [b for b in d.find_elements(By.CSS_SELECTOR, '.event-block') if 'Тестовое совещание' in b.get_attribute('textContent')][0]
    # Блок может частично перекрываться линией «сейчас» — клик напрямую.
    d.execute_script('arguments[0].scrollIntoView({block: "center"}); arguments[0].click()', ev)
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

    # Карточка подробно: подзадачи (чек-лист в описании), исполнители, метки, комментарии, вложения.
    def tile(title):
        return [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if title in c.get_attribute('textContent')][0]
    def card1():
        return next(c for c in nc_mock('GET', '/test/state')['cards'] if c['id'] == 1)
    assert '1/3' in tile('Обновить RuPost').get_attribute('textContent'); ok('card tile shows subtask progress 1/3')
    tile('Обновить RuPost').click()
    panel = wait(d, '.deck-card-detail')
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.deck-checklist li')) == 3)
    d.find_element(By.CSS_SELECTOR, '.deck-checklist input[aria-label="Обновить пакеты"]').click()
    W(d, 10).until(lambda d: '- [x] Обновить пакеты' in card1()['description'])
    W(d, 10).until(lambda d: '2/3' in tile('Обновить RuPost').get_attribute('textContent'))
    add = d.find_element(By.CSS_SELECTOR, '.deck-tasks .deck-add-item input'); add.send_keys('Сообщить пользователям'); add.send_keys(Keys.ENTER)
    W(d, 10).until(lambda d: card1()['description'].endswith('- [ ] Проверить почту\n- [ ] Сообщить пользователям'))
    d.execute_script("arguments[0].click()", d.find_element(By.CSS_SELECTOR, 'button[aria-label="Удалить подзадачу Проверить почту"]'))
    W(d, 10).until(lambda d: 'Проверить почту' not in card1()['description'] and len(d.find_elements(By.CSS_SELECTOR, '.deck-checklist li')) == 3)
    assert card1()['description'].startswith('Окно работ в среду'), card1()['description']
    ok('subtasks: toggle, add, delete saved to card description')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-people-row .deck-chip.add')][0].click()
    [l for l in W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.deck-picker label')) if 'Петрова Анна' in l.text][0].click()
    W(d, 10).until(lambda d: any(u['participant']['uid'] == 'petrova' for u in card1().get('assignedUsers') or []))
    labels_add = d.find_elements(By.CSS_SELECTOR, '.deck-people-row')[1].find_element(By.CSS_SELECTOR, '.deck-chip.add')
    labels_add.click()
    [l for l in W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.deck-people-row')[1].find_elements(By.CSS_SELECTOR, '.deck-picker label')) if 'Сеть' in l.text][0].click()
    W(d, 10).until(lambda d: any(l['title'] == 'Сеть' for l in card1().get('labels', [])))
    ok('assignee and label added')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-tabs button') if 'Комментарии' in b.text][0].click()
    W(d, 10).until(lambda d: 'Согласовано с ИБ' in d.find_element(By.CSS_SELECTOR, '.deck-comments').get_attribute('textContent'))
    ci = d.find_element(By.CSS_SELECTOR, '.deck-comments .deck-add-item input'); ci.send_keys('Начинаем в 20:00'); ci.send_keys(Keys.ENTER)
    W(d, 10).until(lambda d: any(c['message'] == 'Начинаем в 20:00' for c in nc_mock('GET', '/test/state')['comments']['1']))
    W(d, 10).until(lambda d: 'Начинаем в 20:00' in d.find_element(By.CSS_SELECTOR, '.deck-comments').get_attribute('textContent'))
    ok('card comments listed and added')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-tabs button') if 'Вложения' in b.text][0].click()
    W(d, 10).until(lambda d: 'Регламент.pdf' in d.find_element(By.CSS_SELECTOR, '.deck-files').get_attribute('textContent'))
    ok('card attachments listed')
    d.save_screenshot(f'{OUT}/10a-deck-card.png')
    d.find_element(By.CSS_SELECTOR, '.deck-panel button[aria-label="Закрыть"]').click()
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
    # Фильтр карточек: исполнитель, метка, сброс.
    from selenium.webdriver.support.ui import Select
    shown = lambda: sorted(c.find_element(By.CSS_SELECTOR, '.deck-card-title').get_attribute('textContent').strip() for c in d.find_elements(By.CSS_SELECTOR, '.deck-columns .deck-card'))
    Select(d.find_element(By.CSS_SELECTOR, '.deck-filters select[aria-label="Исполнитель"]')).select_by_visible_text('Мои карточки')
    W(d, 5).until(lambda d: shown() == ['Настроить брокер X2Go'])
    Select(d.find_element(By.CSS_SELECTOR, '.deck-filters select[aria-label="Исполнитель"]')).select_by_visible_text('Иванов Иван')
    W(d, 5).until(lambda d: shown() == ['Обновить RuPost'])
    Select(d.find_element(By.CSS_SELECTOR, '.deck-filters select[aria-label="Исполнитель"]')).select_by_visible_text('Без исполнителя')
    W(d, 5).until(lambda d: shown() == ['Проверить резервные копии'])
    Select(d.find_element(By.CSS_SELECTOR, '.deck-filters select[aria-label="Исполнитель"]')).select_by_visible_text('Все исполнители')
    Select(d.find_element(By.CSS_SELECTOR, '.deck-filters select[aria-label="Метка"]')).select_by_visible_text('Срочно')
    W(d, 5).until(lambda d: shown() == ['Обновить RuPost'] and 'Показано 1 из 3' in d.find_element(By.CSS_SELECTOR, '.deck-filters').get_attribute('textContent'))
    d.save_screenshot(f'{OUT}/25-deck-filter.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-filters button') if 'Сбросить' in b.text][0].click()
    W(d, 5).until(lambda d: len(shown()) == 3)
    ok('deck filter: my cards, by assignee, unassigned, by label, reset')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.deck-card .sender-avatar.has-image img'))
    ok('deck: assignee avatars loaded from Nextcloud')
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
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.room-row .sender-avatar.has-image img')) == 2)
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-msg .sender-avatar.has-image img'))
    ok('talk: room and author avatars loaded from Nextcloud')
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

    # Поиск сотрудников из списка бесед: в том числе тех, кого Nextcloud не подсказывает
    # (ограничение автодополнения), — через корпоративную адресную книгу по e-mail.
    assert dav_put('/test@rrcki.ru/contacts/smirnov.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:smirnov\r\nFN:Смирнов Пётр\r\nEMAIL:smirnov@nrcki.ru\r\nEND:VCARD\r\n') in (200, 201, 204)
    rooms_search = d.find_element(By.CSS_SELECTOR, '.talk-rooms .search-box input')
    rooms_search.send_keys('Петров')
    W(d, 15).until(lambda d: any('Петрова Анна' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-people .person-row')))
    clear(rooms_search); rooms_search.send_keys('Смирнов')
    W(d, 20).until(lambda d: any('Смирнов Пётр' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-people .person-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-people .person-row') if 'Смирнов Пётр' in r.get_attribute('textContent')][0].click()
    W(d, 15).until(lambda d: 'Смирнов Пётр' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent'))
    dav_delete('/test@rrcki.ru/contacts/smirnov.vcf')
    ok('chat search finds employees in Nextcloud, including via the address book (exact e-mail)')

    # 20. Файлы Nextcloud: папки, публичная ссылка, ссылка в письме, доступ сотруднику, загрузка, удаление.
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Файлы']]").click()
    def file_row(name):
        return [r for r in d.find_elements(By.CSS_SELECTOR, '.files-list .file-row') if name in r.get_attribute('textContent')]
    W(d, 20).until(lambda d: file_row('Документы') and file_row('Talk'))
    file_row('Документы')[0].find_element(By.CSS_SELECTOR, '.file-name.link').click()
    W(d, 15).until(lambda d: file_row('Регламент.txt') and file_row('План.docx'))
    # Euro-Office через Direct Editing: документ — в отдельном окне, без входа и без доступа к командам программы.
    main_h = d.current_window_handle
    def editor_window(expected):
        W(d, 15).until(lambda d: len(d.window_handles) == 2)
        d.switch_to.window([h for h in d.window_handles if h != main_h][0])
        W(d, 15).until(lambda d: expected in d.find_element(By.ID, 'doc').text)
        # Tauri внедряет мост во все окна, но команды для окна редактора запрещены (нет в capabilities).
        res = d.execute_async_script('''const done = arguments[arguments.length - 1];
            const t = window.__TAURI_INTERNALS__;
            if (!t) { done('no-ipc'); return }
            t.invoke('get_profile').then((r) => done('ALLOWED ' + JSON.stringify(r))).catch((e) => done('denied: ' + e))''')
        print('   editor IPC:', res)
        assert not res.startswith('ALLOWED'), res
        # Переход на чужой сайт в окне редактора не выполняется (ссылка уходит в браузер).
        d.execute_script("location.href = 'https://phish.example/login'")
        time.sleep(1.5)
        assert expected in d.find_element(By.ID, 'doc').text and 'phish' not in d.current_url, d.current_url
        d.save_screenshot(f'{OUT}/26-editor.png')
        d.close(); d.switch_to.window(main_h)
    file_row('План.docx')[0].click()
    [b for b in wait(d, '.file-panel').find_elements(By.CSS_SELECTOR, '.file-actions button') if 'Редактировать' in b.text][0].click()
    editor_window('Редактор: /Документы/План.docx')
    ok('docx opened in Euro-Office (Direct Editing) in its own window, no IPC access')
    d.find_element(By.CSS_SELECTOR, '.create-doc > button').click()
    [b for b in wait(d, '.create-doc-menu').find_elements(By.TAG_NAME, 'button') if 'Таблица' in b.text][0].click()
    name_in = wait(d, '.create-doc-form input'); clear(name_in); name_in.send_keys('Бюджет 2027')
    d.find_element(By.CSS_SELECTOR, '.create-doc-form button[type=submit]').click()
    editor_window('Редактор: /Документы/Бюджет 2027.xlsx')
    W(d, 10).until(lambda d: '/Документы/Бюджет 2027.xlsx' in nc_mock('GET', '/test/state')['files'] and file_row('Бюджет 2027.xlsx'))
    ok('new spreadsheet created through the editor and opened')
    file_row('Регламент.txt')[0].click()
    panel = wait(d, '.file-panel')
    [b for b in panel.find_elements(By.CSS_SELECTOR, '.share-add')][0].click()
    [b for b in wait(d, '.share-form').find_elements(By.TAG_NAME, 'button') if 'Создать ссылку' in b.text][0].click()
    W(d, 15).until(lambda d: any(x['share_type'] == 3 and x['path'] == '/Документы/Регламент.txt' for x in nc_mock('GET', '/test/state')['shares']))
    W(d, 15).until(lambda d: 'https://localhost:8443/s/tok' in d.find_element(By.CSS_SELECTOR, '.file-panel .share-url').get_attribute('textContent'))
    ok('public link created for a file')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.file-panel .file-actions button') if 'Отправить ссылкой' in b.text][0].click()
    body = W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-body'))
    W(d, 10).until(lambda d: 'https://localhost:8443/s/tok' in d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-body').get_attribute('innerHTML'))
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'Регламент.txt'
    links = [x for x in nc_mock('GET', '/test/state')['shares'] if x['share_type'] == 3 and x['path'] == '/Документы/Регламент.txt']
    # Ссылка без срока для письма не берётся — создаётся новая со сроком 30 дней (политика по умолчанию).
    assert len(links) == 2 and links[1]['expiration'], links
    import datetime as _dt
    assert links[1]['expiration'][:10] == (_dt.date.today() + _dt.timedelta(days=30)).isoformat(), links[1]
    ok('«Отправить ссылкой»: new mail with a link that expires in 30 days')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.compose-modal .compose-header button') if b.get_attribute('aria-label') == 'Закрыть'][0].click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.file-panel .file-actions button') if 'Отправить ссылкой' in b.text][0].click()
    W(d, 10).until(lambda d: 'https://localhost:8443/s/tok' in d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-body').get_attribute('innerHTML'))
    assert len([x for x in nc_mock('GET', '/test/state')['shares'] if x['share_type'] == 3 and x['path'] == '/Документы/Регламент.txt']) == 2, 'link with expiry must be reused'
    ok('second «Отправить ссылкой» reuses the link with expiry')
    # В том же письме — «Ссылка Nextcloud» из окна письма.
    d.find_element(By.CSS_SELECTOR, '.nc-link-button').click()
    W(d, 10).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.nc-picker-list .file-row') if 'Документы' in r.get_attribute('textContent')])
    [r for r in d.find_elements(By.CSS_SELECTOR, '.nc-picker-list .file-row') if 'Документы' in r.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.nc-picker-list .file-row') if 'Регламент.txt' in r.get_attribute('textContent')])
    [r for r in d.find_elements(By.CSS_SELECTOR, '.nc-picker-list .file-row') if 'Регламент.txt' in r.get_attribute('textContent')][0].find_element(By.TAG_NAME, 'input').click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.nc-picker button') if 'Вставить' in b.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.nc-picker') and d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-body').get_attribute('innerHTML').count('/s/tok') >= 2)
    ok('composer: link to a Nextcloud file inserted')
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.compose-modal'))
    # Доступ сотруднику.
    file_row('Регламент.txt')[0].click()
    panel = wait(d, '.file-panel')
    panel.find_element(By.CSS_SELECTOR, '.share-section input[placeholder="Имя сотрудника"]').send_keys('Петр')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.file-panel .share-user'))[0].click()
    W(d, 10).until(lambda d: any(x['share_type'] == 0 and x['share_with'] == 'petrova' for x in nc_mock('GET', '/test/state')['shares']))
    W(d, 10).until(lambda d: 'Петрова Анна' in d.find_element(By.CSS_SELECTOR, '.file-panel').get_attribute('textContent'))
    ok('file shared with a colleague')
    # Загрузка в текущую папку и удаление.
    d.find_element(By.CSS_SELECTOR, '.files-upload').click()
    W(d, 20).until(lambda d: '/Документы/отчёт-e2e.txt' in nc_mock('GET', '/test/state')['files'] and file_row('отчёт-e2e.txt'))
    file_row('отчёт-e2e.txt')[0].click()
    [b for b in wait(d, '.file-panel').find_elements(By.CSS_SELECTOR, '.file-actions button') if 'Удалить' in b.text][0].click()
    accept_confirm(d)
    W(d, 15).until(lambda d: '/Документы/отчёт-e2e.txt' not in nc_mock('GET', '/test/state')['files'] and not file_row('отчёт-e2e.txt'))
    ok('file uploaded to the current folder and deleted')
    d.save_screenshot(f'{OUT}/13-files.png')
    # Вложение письма → Nextcloud (/Почта).
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    [f for f in W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, 'button.folder-item')) if 'Входящие' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: any('Отчёт с вложением' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Отчёт с вложением' in r.get_attribute('textContent')][0].click()
    wait(d, '.attachment-cloud').click()
    W(d, 20).until(lambda d: '/Почта/Отчёт за октябрь.txt' in nc_mock('GET', '/test/state')['files'])
    ok('mail attachment saved to Nextcloud /Почта')

    # Удаление и перемещение писем.
    def utf7(name):
        import base64 as _b
        out, buf = [], ''
        for ch in name + '\0':
            if ch != '\0' and 0x20 <= ord(ch) <= 0x7e or ch == '\0':
                if buf: out.append('&' + _b.b64encode(buf.encode('utf-16-be')).decode().rstrip('=').replace('/', ',') + '-'); buf = ''
                if ch != '\0': out.append('&-' if ch == '&' else ch)
            else:
                buf += ch
        return ''.join(out)
    from email.message import EmailMessage
    from selenium.webdriver.common.action_chains import ActionChains
    def imap():
        ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
        M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('test@rrcki.ru', 'Secret-123'); return M
    def subjects(folder):
        M = imap(); M.select(f'"{folder}"')
        _, ids = M.search(None, 'ALL'); out = []
        for i in ids[0].split():
            _, data = M.fetch(i, '(BODY.PEEK[HEADER.FIELDS (SUBJECT)])')
            out.append(str(make_header(decode_header(email.message_from_bytes(data[0][1])['Subject'] or ''))))
        M.logout(); return out
    M = imap()
    for subj in ('E2E перенос A', 'E2E перенос B', 'E2E удалить C', 'E2E тащить D'):
        msg = EmailMessage(); msg['From'] = 'Робот <robot@nrcki.ru>'; msg['To'] = 'test@rrcki.ru'; msg['Subject'] = subj; msg.set_content('тест')
        M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), msg.as_bytes())
    M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    row = lambda t: [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if t in r.get_attribute('textContent')]
    W(d, 20).until(lambda d: row('E2E удалить C') and row('E2E тащить D'))
    row('E2E удалить C')[0].click()
    W(d, 10).until(lambda d: 'E2E удалить C' in d.find_element(By.CSS_SELECTOR, '.reader-title-row').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Удалить письмо"]').click()
    W(d, 10).until(lambda d: not row('E2E удалить C'))
    W(d, 10).until(lambda d: 'E2E удалить C' in subjects('Trash') and 'E2E удалить C' not in subjects('INBOX'))
    ok('delete: message moved to Trash on server, removed from list')
    row('E2E перенос A')[0].click()
    W(d, 10).until(lambda d: 'E2E перенос A' in d.find_element(By.CSS_SELECTOR, '.reader-title-row').get_attribute('textContent'))
    ActionChains(d).key_down(Keys.CONTROL).click(row('E2E перенос B')[0]).key_up(Keys.CONTROL).perform()
    W(d, 5).until(lambda d: 'Отмечено: 2' in d.find_element(By.CSS_SELECTOR, '.bulk-bar').get_attribute('textContent'))
    d.save_screenshot(f'{OUT}/16-multiselect.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.bulk-bar button') if 'Переместить' in b.text][0].click()
    picker = wait(d, '.folder-picker')
    picker.find_element(By.CSS_SELECTOR, 'input').send_keys('2026')
    W(d, 5).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.folder-picker-list .folder-item')) == 1)
    d.find_element(By.CSS_SELECTOR, '.folder-picker-list .folder-item').click()
    W(d, 10).until(lambda d: not row('E2E перенос A') and not row('E2E перенос B'))
    proj = utf7('Проекты/2026')
    W(d, 10).until(lambda d: {'E2E перенос A', 'E2E перенос B'} <= set(subjects(proj)))
    ok('multi-select (Ctrl+click) and move to folder via picker')
    # Перетаскивание письма на папку «Архив»/«Спам» в боковой панели.
    target = [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Спам' in f.get_attribute('textContent')][0]
    src = row('E2E тащить D')[0]
    ac = ActionChains(d).move_to_element(src).click_and_hold()
    for i in range(1, 6):
        ac = ac.move_to_element_with_offset(target, 0, 0) if i == 5 else ac.move_by_offset(-20 * i, 3)
    ac.perform()
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.mail-drag-ghost'))
    d.save_screenshot(f'{OUT}/17-drag.png')
    ActionChains(d).release().perform()
    W(d, 10).until(lambda d: not row('E2E тащить D'))
    W(d, 10).until(lambda d: 'E2E тащить D' in subjects('Junk'))
    ok('drag message onto sidebar folder moves it')
    # Из корзины — безвозвратно, с подтверждением; клавиша Del.
    def flags_of(folder, subj):
        M = imap(); M.select(f'"{folder}"')
        _, ids = M.search(None, 'SUBJECT', f'"{subj}"')
        if not ids[0]: M.logout(); return None
        _, data = M.fetch(ids[0].split()[-1], '(FLAGS)'); M.logout(); return data[0].decode()
    # Письмо, помеченное к удалению «другим клиентом», не должно исчезнуть при нашем удалении.
    msg = EmailMessage(); msg['From'] = 'x@nrcki.ru'; msg['To'] = 'test@rrcki.ru'; msg['Subject'] = 'E2E other client deleted'; msg.set_content('x')
    M = imap(); M.append('Trash', '(\\Deleted)', imaplib.Time2Internaldate(time.time()), msg.as_bytes()); M.logout()
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Корзина' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: row('E2E удалить C'))
    row('E2E удалить C')[0].click()
    W(d, 10).until(lambda d: 'E2E удалить C' in d.find_element(By.CSS_SELECTOR, '.reader-title-row').get_attribute('textContent'))
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.DELETE)
    wait(d, '.confirm-modal')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.confirm-modal button') if 'Удалить' in b.text][0].click()
    W(d, 10).until(lambda d: not row('E2E удалить C'))
    W(d, 10).until(lambda d: 'E2E удалить C' not in subjects('Trash'))
    assert 'E2E other client deleted' in subjects('Trash'), 'EXPUNGE must not remove messages deleted by another client'
    assert '\\Deleted' in flags_of('Trash', 'E2E other client deleted')
    ok('Del in Trash: confirmation, permanently expunged; other client\'s \\Deleted message kept (no UIDPLUS)')

    # Ответить всем, «Важное», архив, отдельное окно письма.
    M = imap()
    for subj in ('E2E reply-all', 'E2E window'):
        msg = EmailMessage(); msg['From'] = 'Начальник <boss@nrcki.ru>'; msg['To'] = 'test@rrcki.ru, Петров <petrov@nrcki.ru>'
        msg['Cc'] = 'Сидоров <sidorov@nrcki.ru>, test@rrcki.ru'; msg['Subject'] = subj; msg['Message-ID'] = f'<{subj.replace(" ", "-")}@nrcki.ru>'; msg.set_content('Длинный текст письма.\n' * 200)
        M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), msg.as_bytes())
    M.logout()
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Входящие' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: row('E2E reply-all') and row('E2E window'))
    row('E2E reply-all')[0].click()
    W(d, 10).until(lambda d: 'E2E reply-all' in d.find_element(By.CSS_SELECTOR, '.reader-title-row').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Ответить всем"]').click()
    wait(d, '.compose-modal')
    fields = d.find_elements(By.CSS_SELECTOR, '.compose-modal .recipient-field input')
    to_v, cc_v = fields[0].get_attribute('value'), fields[1].get_attribute('value')
    assert 'boss@nrcki.ru' in to_v and 'petrov@nrcki.ru' in to_v and 'test@rrcki.ru' not in to_v, to_v
    assert 'sidorov@nrcki.ru' in cc_v and 'test@rrcki.ru' not in cc_v, cc_v
    assert 'Re: E2E reply-all' == d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value')
    ok('reply all: sender + recipients, own address excluded, Cc kept')
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Важное"]').click()
    W(d, 10).until(lambda d: '$label1' in (flags_of('INBOX', 'E2E reply-all') or ''))
    W(d, 5).until(lambda d: row('E2E reply-all')[0].find_elements(By.CSS_SELECTOR, '.important-mark'))
    ok('important: $label1 keyword set on server, mark in list')
    d.save_screenshot(f'{OUT}/20-reader-toolbar.png')
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Архивировать"]').click()
    W(d, 15).until(lambda d: not row('E2E reply-all'))
    W(d, 10).until(lambda d: 'E2E reply-all' in subjects('Archive'))
    W(d, 10).until(lambda d: any('Архив' in f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item')))
    ok('archive: «Archive» created on server, message moved')
    # Отдельное окно письма.
    main = d.current_window_handle
    row('E2E window')[0].click()
    W(d, 10).until(lambda d: 'E2E window' in d.find_element(By.CSS_SELECTOR, '.reader-title-row').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Открыть в новом окне"]').click()
    W(d, 15).until(lambda d: len(d.window_handles) == 2)
    d.switch_to.window([h for h in d.window_handles if h != main][0])
    W(d, 20).until(lambda d: 'E2E window' in d.find_element(By.CSS_SELECTOR, '.message-window .reader-title-row').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.message-window button[aria-label="Крупнее"]').click()
    assert d.execute_script("return getComputedStyle(document.querySelector('.message-window .message-body')).fontSize") == '16px'
    d.save_screenshot(f'{OUT}/21-message-window.png')
    d.find_element(By.CSS_SELECTOR, '.message-window button[aria-label="Флажок"]').click()
    W(d, 10).until(lambda d: '\\Flagged' in (flags_of('INBOX', 'E2E window') or ''))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.message-window-toolbar button') if 'Всем' in b.text][0].click()
    to_v = wait(d, '.compose-modal .recipient-field input').get_attribute('value')
    assert 'boss@nrcki.ru' in to_v and 'petrov@nrcki.ru' in to_v, to_v
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    ok('message window: opens separately, font size, flag, reply all')
    d.find_element(By.CSS_SELECTOR, '.message-window button[aria-label="Архивировать"]').click()
    W(d, 15).until(lambda d: len(d.window_handles) == 1)
    d.switch_to.window(main)
    W(d, 10).until(lambda d: 'E2E window' in subjects('Archive'))
    d.execute_script("window.dispatchEvent(new Event('focus'))")
    W(d, 15).until(lambda d: not row('E2E window'))
    ok('message window: archive closes window, main list refreshed')

    # Контекстное меню письма (правая кнопка), как в Outlook.
    M = imap()
    for subj in ('E2E ctx one', 'E2E ctx two'):
        msg = EmailMessage(); msg['From'] = 'Коллега <colleague@nrcki.ru>'; msg['To'] = 'test@rrcki.ru'; msg['Subject'] = subj; msg.set_content('текст')
        M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), msg.as_bytes())
    M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: row('E2E ctx one') and row('E2E ctx two'))
    def menu_pick(r, label):
        ActionChains(d).context_click(r).perform()
        items = W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.context-menu button'))
        [b for b in items if b.find_element(By.CSS_SELECTOR, '.context-label').text == label][0].click()
        W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.context-menu'))
    ActionChains(d).context_click(row('E2E ctx one')[0]).perform()
    labels = [b.find_element(By.CSS_SELECTOR, '.context-label').text for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button')]
    assert {'Ответить', 'Ответить всем', 'Переслать', 'Удалить', 'В архив', 'Пометить как важное', 'Поставить флажок'} <= set(labels), labels
    d.save_screenshot(f'{OUT}/27-context-menu.png')
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ESCAPE)
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.context-menu'))
    menu_pick(row('E2E ctx one')[0], 'Ответить')
    W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'Re: E2E ctx one')
    assert 'colleague@nrcki.ru' in d.find_element(By.CSS_SELECTOR, '.compose-modal .recipient-field input').get_attribute('value')
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    menu_pick(row('E2E ctx one')[0], 'Пометить как важное')
    W(d, 10).until(lambda d: '$label1' in (flags_of('INBOX', 'E2E ctx one') or ''))
    ok('context menu: items like Outlook; reply to an unopened message; mark important')
    row('E2E ctx one')[0].click()
    ActionChains(d).key_down(Keys.CONTROL).click(row('E2E ctx two')[0]).key_up(Keys.CONTROL).perform()
    menu_pick(row('E2E ctx two')[0], 'Удалить (2)')
    W(d, 10).until(lambda d: not row('E2E ctx one') and not row('E2E ctx two'))
    W(d, 10).until(lambda d: {'E2E ctx one', 'E2E ctx two'} <= set(subjects('Trash')))
    ok('context menu on several selected messages: delete both to Trash')
    # После правых щелчков WebKitWebDriver теряет Shift при вводе — дальше поля заполняются напрямую.
    ActionChains(d).reset_actions()

    # Предпросмотр вложений: картинка (миниатюра и просмотр), PDF (pdf.js), текст в cp1251.
    import zlib, struct
    def png(w, h, rgb):
        raw = b''.join(b'\x00' + bytes(rgb) * w for _ in range(h))
        chunk = lambda t, data: struct.pack('>I', len(data)) + t + data + struct.pack('>I', zlib.crc32(t + data) & 0xffffffff)
        return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
    def pdf():
        objs = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
                b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
                None, b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>']
        stream = b'0 0 1 rg 20 20 260 160 re f BT /F1 24 Tf 1 1 1 rg 40 90 Td (Hello PDF) Tj ET'
        objs[3] = b'<< /Length %d >>\nstream\n' % len(stream) + stream + b'\nendstream'
        out = b'%PDF-1.4\n'; offs = []
        for i, o in enumerate(objs, 1):
            offs.append(len(out)); out += b'%d 0 obj\n' % i + o + b'\nendobj\n'
        x = len(out)
        out += b'xref\n0 %d\n0000000000 65535 f \n' % (len(objs) + 1) + b''.join(b'%010d 00000 n \n' % o for o in offs)
        return out + b'trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objs) + 1, x)
    msg = EmailMessage(); msg['From'] = 'Фотограф <photo@nrcki.ru>'; msg['To'] = 'test@rrcki.ru'; msg['Subject'] = 'E2E preview'; msg.set_content('Смотрите вложения.')
    msg.add_attachment(png(120, 80, (220, 40, 40)), maintype='image', subtype='png', filename='снимок.png')
    msg.add_attachment(pdf(), maintype='application', subtype='pdf', filename='схема.pdf')
    msg.add_attachment('Отчёт в кодировке 1251\nстрока 2'.encode('cp1251'), maintype='application', subtype='octet-stream', filename='отчёт.txt')
    msg.add_attachment(b'PK fake smeta', maintype='application', subtype='vnd.openxmlformats-officedocument.wordprocessingml.document', filename='Смета.docx')
    M = imap(); M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), msg.as_bytes()); M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: row('E2E preview'))
    row('E2E preview')[0].click()
    thumb = W(d, 20).until(lambda d: d.find_element(By.CSS_SELECTOR, '.attachment-thumb'))
    W(d, 10).until(lambda d: d.execute_script('return arguments[0].naturalWidth', thumb) == 120)
    ok('attachments: image thumbnail shown without download')
    d.find_element(By.CSS_SELECTOR, '.attachment-item.has-thumb .attachment-card').click()
    img = wait(d, '.viewer-image')
    W(d, 10).until(lambda d: d.execute_script('return arguments[0].naturalWidth', d.find_element(By.CSS_SELECTOR, '.viewer-image')) == 120)
    d.save_screenshot(f'{OUT}/22-viewer-image.png')
    ok('viewer: image opened full-size')
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ARROW_RIGHT)
    W(d, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.pdf-page') or d.find_elements(By.XPATH, "//*[contains(text(),'PDF не открылся')]"))
    canvas = d.find_element(By.CSS_SELECTOR, '.pdf-page')
    time.sleep(1)
    blue = d.execute_script("const c = arguments[0], x = c.getContext('2d').getImageData(Math.floor(c.width * 0.15), Math.floor(c.height * 0.5), 1, 1).data; return [x[0], x[1], x[2]]", canvas)
    assert blue[2] > 200 and blue[0] < 60, blue
    d.save_screenshot(f'{OUT}/23-viewer-pdf.png')
    ok('viewer: PDF rendered by pdf.js')
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ARROW_RIGHT)
    W(d, 10).until(lambda d: 'Отчёт в кодировке 1251' in d.find_element(By.CSS_SELECTOR, '.viewer-text').get_attribute('textContent'))
    ok('viewer: text file, cp1251 decoded')
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ESCAPE)
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop'))
    assert d.find_elements(By.CSS_SELECTOR, '.reader-title-row')
    ok('viewer: Esc closes preview, message stays open')
    main_h = d.current_window_handle
    d.find_element(By.CSS_SELECTOR, '.attachment-edit').click()
    W(d, 15).until(lambda d: len(d.window_handles) == 2)
    d.switch_to.window([h for h in d.window_handles if h != main_h][0])
    W(d, 15).until(lambda d: 'Редактор: /Почта/Смета.docx' in d.find_element(By.ID, 'doc').text)
    d.close(); d.switch_to.window(main_h)
    assert nc_mock('GET', '/test/state')['files']['/Почта/Смета.docx'] == 'PK fake smeta'
    ok('docx attachment: saved to Nextcloud /Почта and opened in the editor')

    # HTML-рассылка: встроенный логотип (cid) внутри письма, а не вложением; картинка из интернета — по кнопке.
    from email.mime.multipart import MIMEMultipart
    from email.mime.image import MIMEImage
    from email.mime.text import MIMEText
    rel = MIMEMultipart('related')
    rel.attach(MIMEText('<html><head><style>.hdr { background: #003366; color: #ffffff; padding: 12px }</style></head><body>'
                        '<table width="600" bgcolor="#f0f0f0"><tr><td class="hdr"><img src="cid:logo@rrcki" width="120" alt="logo"> Новости НИЦ</td></tr>'
                        '<tr><td><p style="color:#990000">Вёрстка <b>рассылки</b></p><img id="remote" src="https://localhost:8443/e2e/remote.png" alt="r">'
                        '<p><a href="https://example.org/novosti">Подробнее</a></p><p><a id="phish" href="https://evil.example/login">https://www.nrcki.ru/login</a></p></td></tr></table></body></html>', 'html', 'utf-8'))
    logo = MIMEImage(png(120, 40, (0, 51, 102)), 'png'); logo.add_header('Content-ID', '<logo@rrcki>'); logo.add_header('Content-Disposition', 'inline', filename='logo.png')
    rel.attach(logo)
    rel['Subject'] = 'E2E html'; rel['From'] = 'Рассылка <news@nrcki.ru>'; rel['To'] = 'test@rrcki.ru'
    M = imap(); M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), rel.as_bytes()); M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: row('E2E html'))
    row('E2E html')[0].click()
    frame = W(d, 20).until(lambda d: d.find_element(By.CSS_SELECTOR, 'iframe.message-html'))
    assert not d.find_elements(By.CSS_SELECTOR, '.attachment-card'), 'inline logo must not be listed as attachment'
    W(d, 10).until(lambda d: 'скрыты (1)' in d.find_element(By.CSS_SELECTOR, '.html-bar.remote').get_attribute('textContent'))
    assert nc_mock('GET', '/test/state').get('remote_png', 0) == 0, 'remote image must not load by itself'
    d.switch_to.frame(frame)
    W(d, 10).until(lambda d: d.execute_script("const i = document.querySelector('img[alt=logo]'); return i && i.complete && i.naturalWidth") == 120)
    assert d.execute_script("return getComputedStyle(document.querySelector('.hdr')).backgroundColor") == 'rgb(0, 51, 102)'
    assert d.execute_script("return document.querySelector('img[alt=r]') !== null && !document.querySelector('img[alt=r]').getAttribute('src')")
    d.switch_to.default_content()
    h = d.execute_script("return arguments[0].getBoundingClientRect().height", frame)
    assert h > 120, h
    d.save_screenshot(f'{OUT}/24-html-mail.png')
    ok('HTML mail: styles and inline cid logo inside the letter, not an attachment; remote image blocked')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.html-bar.remote button') if 'Показать' in b.text][0].click()
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.html-bar.remote'))
    frame = d.find_element(By.CSS_SELECTOR, 'iframe.message-html'); d.switch_to.frame(frame)
    W(d, 10).until(lambda d: d.execute_script("const i = document.querySelector('img[alt=r]'); return i && i.complete && i.naturalWidth") == 40)
    d.switch_to.default_content()
    assert nc_mock('GET', '/test/state').get('remote_png', 0) == 1
    ok('HTML mail: «Показать картинки» loads remote image via the app')
    frame = d.find_element(By.CSS_SELECTOR, 'iframe.message-html'); d.switch_to.frame(frame)
    d.execute_script("document.querySelector('a[href=\"https://evil.example/login\"]').click()")
    d.switch_to.default_content()
    modal = W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.confirm-modal[aria-label="Подозрительная ссылка"]'))
    t = modal.get_attribute('textContent')
    assert 'nrcki.ru' in t and 'evil.example' in t, t
    d.save_screenshot(f'{OUT}/28-phishing-link.png')
    [b for b in modal.find_elements(By.TAG_NAME, 'button') if 'Не открывать' in b.text][0].click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.confirm-modal'))
    ok('link whose text shows another domain: warning before opening')
    # Даже если бы очистку обошли — CSP рамки письма не даёт исполнить код (обработчик onerror).
    frame = d.find_element(By.CSS_SELECTOR, 'iframe.message-html'); d.switch_to.frame(frame)
    d.execute_script("document.body.insertAdjacentHTML('beforeend', '<img src=\"x\" onerror=\"window.parent.__xss = 1\">')")
    d.switch_to.default_content(); time.sleep(1)
    assert d.execute_script('return window.__xss') is None, 'inline handler must be blocked by the frame CSP'
    ok('mail frame CSP blocks injected inline handlers')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.html-foot button') if 'как текст' in b.text][0].click()
    W(d, 5).until(lambda d: 'Вёрстка рассылки' in d.find_element(By.CSS_SELECTOR, '.message-body.plain').get_attribute('textContent'))
    ok('HTML mail: plain-text view toggle')

    # Уведомления о новой почте и сообщениях чата.
    notify_log = os.environ['RRCKI_E2E_NOTIFY_LOG']
    def notified():
        return open(notify_log, encoding='utf-8').read() if os.path.exists(notify_log) else ''
    def new_mail(subj):
        m = EmailMessage(); m['From'] = 'Коллега <colleague@nrcki.ru>'; m['To'] = 'test@rrcki.ru'; m['Subject'] = subj; m.set_content('x')
        M = imap(); M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), m.as_bytes()); M.logout()
    # После закрытых окон редактора в Xvfb (без оконного менеджера) фокус не возвращается — задаём явно.
    d.execute_script('document.hasFocus = () => true')
    new_mail('E2E notify in-app')
    try:
        W(d, 40).until(lambda d: any('Новое письмо от Коллега: E2E notify in-app' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    except Exception:
        print('DIAG notified:', notified()[-400:], 'hidden/focus:', d.execute_script('return [document.hidden, document.hasFocus(), document.visibilityState]'), 'toasts:', [t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')])
        raise
    assert 'E2E notify in-app' not in notified(), 'window in front: in-app notice, not a system one'
    ok('new mail while the window is active: in-app notice, list refreshed')
    d.execute_script('document.hasFocus = () => false')
    new_mail('E2E notify system')
    W(d, 40).until(lambda d: 'Новое письмо — Коллега\tE2E notify system' in notified())
    nc_mock('POST', '/test/inject?token=abc123&text=' + __import__('urllib.parse').parse.quote('Уведомление из чата'))
    W(d, 40).until(lambda d: 'Чат: УИТ — общий\tИванов Иван: Уведомление из чата' in notified())
    d.execute_script('delete document.hasFocus')
    print('   notify log:', notified().strip().replace('\n', ' | '))
    ok('window in background: system notifications for new mail and chat message')

    # Фильтры почты (SOGo): адрес сервера SOGo — мок на 8444.
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    dav = [i for i in d.find_elements(By.CSS_SELECTOR, '.settings-modal .login-field input') if 'автоопределение' in (i.get_attribute('placeholder') or '')][0]
    set_value(d, dav, 'https://localhost:8444/')
    # Смена адреса сервера требует пароль почты; без него — отказ.
    save_btn = [b for b in d.find_elements(By.CSS_SELECTOR, '.settings-modal button') if b.text == 'Сохранить'][0]
    assert not save_btn.is_enabled()
    set_value(d, wait(d, '.settings-modal input[aria-label="Пароль для подтверждения"]'), 'wrong')
    save_btn.click()
    W(d, 10).until(lambda d: any('введите пароль почты' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    set_value(d, d.find_element(By.CSS_SELECTOR, '.settings-modal input[aria-label="Пароль для подтверждения"]'), 'Secret-123')
    save_btn.click()
    W(d, 10).until(lambda d: any('Настройки сохранены' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    ok('changing server addresses requires the mail password')
    set_value(d, dav, 'http://localhost:8444/')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.settings-modal input[aria-label="Пароль для подтверждения"]'), 'Secret-123')
    save_btn.click()
    W(d, 10).until(lambda d: any('только https' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    set_value(d, dav, 'https://localhost:8444/')
    ok('http address for calendar/contacts refused')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.settings-modal button') if 'Фильтры почты' in b.text][0].click()
    W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.filter-item'))
    t = d.find_element(By.CSS_SELECTOR, '.filter-item').get_attribute('textContent')
    assert 'Рассылка' in t and '[noc]' in t, t; ok('filters: existing SOGo filter listed')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filters-modal button') if 'Новый фильтр' in b.text][0].click()
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Название фильтра"]'), 'От Иванова')
    from selenium.webdriver.support.ui import Select
    Select(d.find_element(By.CSS_SELECTOR, '.filter-row select[aria-label="Поле"]')).select_by_visible_text('От')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.filter-row input[aria-label="Значение"]'), 'ivanov@nrcki.ru')
    d.find_element(By.CSS_SELECTOR, '.filter-folder').click()
    wait(d, '.folder-picker input').send_keys('2026')
    W(d, 5).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.folder-picker-list .folder-item')) == 1)
    d.find_element(By.CSS_SELECTOR, '.folder-picker-list .folder-item').click()
    d.save_screenshot(f'{OUT}/18-filter-edit.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-editor button') if 'Сохранить фильтр' in b.text][0].click()
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.filter-item')) == 2)
    import json as _j, urllib.request as _u
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    saved = _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())
    f2 = saved['SOGoSieveFilters'][1]
    assert saved['SOGoMailSignature'] == 'С уважением', saved
    assert f2['name'] == 'От Иванова' and f2['rules'] == [{'field': 'from', 'operator': 'contains', 'value': 'ivanov@nrcki.ru'}], f2
    assert f2['actions'] == [{'method': 'fileinto', 'argument': 'Проекты/2026'}], f2
    ok('filters: new rule saved to SOGo, other preferences kept')
    d.find_element(By.CSS_SELECTOR, '.filter-item .switch input').click()
    W(d, 10).until(lambda d: _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())['SOGoSieveFilters'][0]['active'] == 0)
    ok('filters: toggle off saved')
    d.save_screenshot(f'{OUT}/19-filters.png')
    d.find_element(By.CSS_SELECTOR, '.filters-modal button[aria-label="Закрыть"]').click()
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14a. письма на компьютере, поиск по всей почте, адреса, подпись
d = session()
try:
    wait(d, '.message-row', 30)
    def inv(cmd, args=None):
        return d.execute_async_script("const cb = arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {}).then((r) => cb({ok: r}), (e) => cb({err: JSON.stringify(e)}))", cmd, args)
    # фоновая синхронизация: индекс всех папок и исходники писем за 7 дней
    W(d, 90).until(lambda d: (inv('offline_status').get('ok') or {}).get('cached', 0) >= 60 and not inv('offline_status')['ok']['syncing'])
    st = inv('offline_status')['ok']
    assert st['enabled'] and st['days'] == 7 and st['messages'] >= 100, st
    ok(f"local store: {st['messages']} messages indexed, {st['cached']} stored (7 days)")
    store_files = [f for f in glob.glob(os.environ['XDG_DATA_HOME'] + '/**/mailstore/**/*', recursive=True) if os.path.isfile(f)]
    assert len(store_files) > 70, len(store_files)
    blob = b''.join(open(f, 'rb').read() for f in store_files)
    for secret in ['Тестовое письмо'.encode(), 'robot@rrcki.ru'.encode(), 'Отчёт с вложением'.encode(), 'Содержимое отчёта'.encode()]:
        assert secret not in blob, secret
    ok('local store on disk is encrypted (no subjects, addresses or bodies in plaintext)')
    page = inv('cached_messages', {'folder': 'INBOX', 'beforeUid': None, 'limit': 5})['ok']
    assert page and len(page['items']) == 5, page
    ok('cached first page available instantly')

    # поиск: по тексту хранимых писем, по всем папкам, кнопка очистки
    q = d.find_element(By.CSS_SELECTOR, 'input[aria-label="Поиск писем"]')
    set_value(d, q, 'номер 119')
    subj = lambda d: [r.find_element(By.CSS_SELECTOR, '.message-subject').get_attribute('textContent').strip() for r in d.find_elements(By.CSS_SELECTOR, '.message-row')]
    try:
        W(d, 10).until(lambda d: subj(d) == ['Письмо №119'])
    except Exception:
        print('ROWS:', subj(d)); raise
    ok('search finds text inside locally stored mail')
    set_value(d, q, 'проектной')
    W(d, 10).until(lambda d: subj(d) == ['В проектной папке'])
    assert '2026' in d.find_element(By.CSS_SELECTOR, '.message-row .message-folder').get_attribute('textContent')
    d.find_element(By.CSS_SELECTOR, '.message-row').click()
    W(d, 15).until(lambda d: 'Письмо в папке с русским именем' in d.find_element(By.CSS_SELECTOR, '.reader-content').text)
    ok('search covers all folders; result from another folder opens')
    set_value(d, q, 'номер 5.')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.search-header'))
    assert 'Письмо №5' not in subj(d), subj(d)
    [b for b in d.find_elements(By.CSS_SELECTOR, '.search-header button') if 'старых писем' in b.text][0].click()
    W(d, 30).until(lambda d: 'Письмо №5' in subj(d))
    ok('server search finds older mail by its text')
    d.save_screenshot(f'{OUT}/42-search.png')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Очистить поиск"]').click()
    W(d, 10).until(lambda d: q.get_attribute('value') == '' and not d.find_elements(By.CSS_SELECTOR, '.search-header') and len(d.find_elements(By.CSS_SELECTOR, '.message-row')) >= 50)
    set_value(d, q, 'что-то')
    q.send_keys(Keys.ESCAPE)
    W(d, 5).until(lambda d: q.get_attribute('value') == '')
    ok('search clear button and Esc reset the search')

    # подпись: из веб-почты SOGo, изменение сохраняется в SOGo и попадает в письмо
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sig = wait(d, 'textarea[aria-label="Подпись"]')
    W(d, 10).until(lambda d: sig.get_attribute('value') == 'С уважением')
    set_value(d, sig, 'С уважением,\nТестов Тест\nОтдел испытаний')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.signature-settings button') if 'Сохранить подпись' in b.text][0].click()
    import json as _j, urllib.request as _u
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    W(d, 10).until(lambda d: 'Отдел испытаний' in _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())['SOGoMailSignature'])
    assert _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())['SOGoSieveFilters'], 'filters lost'
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    body = wait(d, '.compose-body')
    try:
        W(d, 5).until(lambda d: 'Отдел испытаний' in body.get_attribute('textContent') and '--' in body.get_attribute('textContent'))
    except Exception:
        print('BODY:', repr(body.get_attribute('innerHTML'))); raise
    assert body.get_attribute('spellcheck') == 'true'
    body.click()
    d.execute_script("arguments[0].focus(); const r = document.createRange(); r.setStart(arguments[0], 0); r.collapse(true); const s = getSelection(); s.removeAllRanges(); s.addRange(r); document.execCommand('insertText', false, 'Превет, это тестт ')", body)
    time.sleep(1)
    d.save_screenshot(f'{OUT}/44-signature-spellcheck.png')
    ok('signature: taken from SOGo, saved back to SOGo, inserted into new mail; editor has spellcheck')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[aria-label="Закрыть"]').click()
    # контакты — снова на CardDAV-сервере (выше адрес указывал на мок SOGo)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    dav = [i for i in wait(d, '.profile-form').find_elements(By.CSS_SELECTOR, 'input') if 'автоопределение' in (i.get_attribute('placeholder') or '')][0]
    set_value(d, dav, 'https://localhost:5232/')
    set_value(d, wait(d, 'input[aria-label="Пароль для подтверждения"]'), 'Secret-123')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.profile-form button') if b.text == 'Сохранить'][0].click()
    W(d, 15).until(lambda d: (inv('get_profile').get('ok') or {}).get('davUrl') == 'https://localhost:5232/')
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
    # адрес отправителя: копировать, в контакты
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Отчёт с вложением' in r.get_attribute('textContent')][0].click()
    chip = wait(d, '.sender-line .address-chip')
    assert chip.get_attribute('data-address') == 'yushkov_do@nrcki.ru', chip.get_attribute('data-address')
    assert d.execute_script("const s = getComputedStyle(arguments[0]); return s.userSelect || s.webkitUserSelect", chip) in ('text', 'auto'), 'not selectable'
    chip.click()
    [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if b.text == 'Копировать адрес'][0].click()
    W(d, 5).until(lambda d: 'Скопировано: yushkov_do@nrcki.ru' in d.find_element(By.CSS_SELECTOR, '.toast').get_attribute('textContent'))
    chip.click()
    [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if 'Добавить в контакты' in b.text][0].click()
    ed = wait(d, '.editor-modal', 20)
    assert field(d, 'Фамилия Имя Отчество').get_attribute('value') == 'Юшков Дмитрий'
    assert 'yushkov_do@nrcki.ru' in field(d, 'E-mail (по одному в строке)').get_attribute('value')
    d.save_screenshot(f'{OUT}/43-add-contact.png')
    ed.find_element(By.CSS_SELECTOR, 'button[type=submit]').click()
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.editor-modal'))
    W(d, 10).until(lambda d: any('yushkov_do@nrcki.ru' in c['emails'] for c in (inv('list_contacts').get('ok') or {}).get('contacts', [])))
    ok('sender address: copy and add to contacts (prefilled)')

except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14b. защита кодом (TOTP): включение, блокировка, разблокировка, резервный код
import hmac as _hmac, hashlib as _hashlib, base64 as _b64, struct as _struct
TOTP = {'secret': None, 'last': -1, 'recovery': []}
def totp_code():
    """Следующий ещё не использованный код (программа не принимает один шаг дважды)."""
    while True:
        now = int(time.time()) // 30
        step = max(now - 1, TOTP['last'] + 1)
        if step <= now + 1:
            break
        time.sleep(2)
    TOTP['last'] = step
    key = _b64.b32decode(TOTP['secret'] + '=' * (-len(TOTP['secret']) % 8))
    h = _hmac.new(key, _struct.pack('>Q', step), _hashlib.sha1).digest()
    o = h[19] & 15
    return '%06d' % ((_struct.unpack('>I', h[o:o + 4])[0] & 0x7fffffff) % 1000000)

def unlock(d, code):
    inp = wait(d, '.lock-card input')
    set_value(d, inp, code)
    d.find_element(By.CSS_SELECTOR, '.lock-card .login-submit').click()

d = session()
try:
    wait(d, '.message-row', 30)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sec = wait(d, '.totp-settings')
    [b for b in sec.find_elements(By.TAG_NAME, 'button') if 'Включить защиту кодом' in b.text][0].click()
    qr = wait(d, '.totp-qr')
    assert qr.get_attribute('src').startswith('data:image/svg+xml'), qr.get_attribute('src')[:60]
    assert d.execute_script('return arguments[0].naturalWidth', qr) > 0, 'QR not rendered'
    TOTP['secret'] = d.find_element(By.CSS_SELECTOR, '.totp-secret').get_attribute('data-secret')
    assert len(TOTP['secret']) == 32, TOTP['secret']
    ok('totp: setup shows QR code and manual key')
    inp = d.find_element(By.CSS_SELECTOR, 'input[aria-label="Код подтверждения"]')
    set_value(d, inp, '000000' if totp_code() != '000000' else '111111')
    d.find_element(By.CSS_SELECTOR, '.totp-setup .primary-button').click()
    W(d, 10).until(lambda d: 'Неверный код' in d.find_element(By.CSS_SELECTOR, '.totp-settings').text)
    TOTP['last'] -= 1
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Код подтверждения"]'), totp_code())
    d.find_element(By.CSS_SELECTOR, '.totp-setup .primary-button').click()
    codes = wait(d, '.totp-codes pre').text.split()
    assert len(codes) == 8 and all(len(c) == 9 and c[4] == '-' for c in codes), codes
    TOTP['recovery'] = codes
    d.save_screenshot(f'{OUT}/40-totp-codes.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.totp-codes button') if 'сохранил' in b.text][0].click()
    W(d, 10).until(lambda d: 'резервных кодов: 8' in d.find_element(By.CSS_SELECTOR, '.totp-settings').text)
    ok('totp: wrong code rejected, right code enables protection, 8 recovery codes shown once')
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
    # Ctrl+L — блокировка: почта отключена, переписки не видно
    from selenium.webdriver.common.action_chains import ActionChains
    d.find_element(By.CSS_SELECTOR, '.message-row').click()
    ActionChains(d).key_down(Keys.CONTROL).send_keys('l').key_up(Keys.CONTROL).perform()
    wait(d, '.lock-card', 15)
    assert not d.find_elements(By.CSS_SELECTOR, '.message-row')
    r = d.execute_async_script("const cb = arguments[0]; window.__TAURI_INTERNALS__.invoke('list_messages', {folder: 'INBOX', beforeUid: null, limit: 5}).then(() => cb('allowed'), (e) => cb('denied:' + JSON.stringify(e)))")
    assert r.startswith('denied'), r
    d.save_screenshot(f'{OUT}/41-totp-locked.png')
    ok('totp: Ctrl+L locks the app, mail commands refused while locked')
    unlock(d, '123456' if totp_code() != '123456' else '654321'); TOTP['last'] -= 1
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.lock-card .login-error'))
    unlock(d, totp_code())
    wait(d, '.message-row', 30)
    ok('totp: wrong code keeps it locked, correct code unlocks and reconnects mail')
    # блокировка из меню и разблокировка резервным кодом
    d.find_element(By.CSS_SELECTOR, '.account-switch').click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.account-menu button') if 'Заблокировать' in b.text][0].click()
    wait(d, '.lock-card', 15)
    [b for b in d.find_elements(By.CSS_SELECTOR, '.lock-card button') if 'резервный код' in b.text][0].click()
    unlock(d, TOTP['recovery'][0].upper())
    wait(d, '.message-row', 30)
    d.find_element(By.CSS_SELECTOR, '.account-switch').click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.account-menu button') if 'Заблокировать' in b.text][0].click()
    wait(d, '.lock-card', 15)
    [b for b in d.find_elements(By.CSS_SELECTOR, '.lock-card button') if 'резервный код' in b.text][0].click()
    unlock(d, TOTP['recovery'][0])
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.lock-card .login-error'))
    ok('totp: recovery code unlocks once and cannot be reused')
finally:
    d.quit()

# 14c. перезапуск с включённой защитой: сохранённый пароль без кода не используется
d = session()
try:
    wait(d, '.lock-card', 30)
    assert not d.find_elements(By.CSS_SELECTOR, '.message-row')
    ok('totp: after restart the app starts locked (saved password not used without code)')
    unlock(d, totp_code())
    wait(d, '.message-row', 30)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sec = wait(d, '.totp-settings')
    assert 'резервных кодов: 7' in sec.text, sec.text
    [b for b in sec.find_elements(By.TAG_NAME, 'button') if 'Отключить защиту' in b.text][0].click()
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Код для отключения"]'), TOTP['recovery'][1])
    [b for b in sec.find_elements(By.TAG_NAME, 'button') if b.text == 'Отключить'][0].click()
    W(d, 10).until(lambda d: 'Включить защиту кодом' in d.find_element(By.CSS_SELECTOR, '.totp-settings').text)
    ok('totp: protection disabled with a recovery code')
finally:
    d.quit()

# 14d. отмена отправки, отложенная отправка, уведомления о прочтении, трей и автозапуск
d = session()
try:
    wait(d, '.message-row', 30)
    def inv(cmd, args=None):
        return d.execute_async_script("const cb = arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {}).then((r) => cb({ok: r}), (e) => cb({err: JSON.stringify(e)}))", cmd, args)
    def compose(to, subject, text):
        d.find_element(By.CSS_SELECTOR, '.compose-top').click()
        wait(d, '.compose-modal')
        set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), to)
        set_value(d, d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'), subject)
        body = d.find_element(By.CSS_SELECTOR, '.compose-body')
        d.execute_script("arguments[0].focus(); const r = document.createRange(); r.setStart(arguments[0], 0); r.collapse(true); const s = getSelection(); s.removeAllRanges(); s.addRange(r); document.execCommand('insertText', false, arguments[1])", body, text)
    def sent_with(subject):
        return [p for p in outbox() if subject_of(p)[0] == subject]

    # Отмена отправки (5 секунд): письмо возвращается в редактор и не уходит.
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    from selenium.webdriver.support.ui import Select
    Select(wait(d, 'select[aria-label="Отмена отправки"]')).select_by_value('5')
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
    compose('colleague@nrcki.ru', 'E2E отмена отправки', 'Текст для отмены')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    toast = wait(d, '.outbox-toast', 10)
    assert 'уйдёт через' in toast.get_attribute('textContent'), toast.get_attribute('textContent')
    d.save_screenshot(f'{OUT}/45-undo-send.png')
    [b for b in toast.find_elements(By.TAG_NAME, 'button') if 'Отменить' in b.text][0].click()
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.compose-modal'))
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'E2E отмена отправки'
    assert 'Текст для отмены' in d.find_element(By.CSS_SELECTOR, '.compose-body').get_attribute('textContent')
    time.sleep(7)
    assert not sent_with('E2E отмена отправки'), 'cancelled mail was sent'
    ok('undo send: countdown, cancel returns the mail to the editor, nothing is sent')
    # Повторная отправка с запросом уведомления о прочтении; «Отправить сейчас» не ждёт отсчёта.
    d.find_element(By.CSS_SELECTOR, '.receipt-toggle input').click()
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    toast = wait(d, '.outbox-toast', 10)
    [b for b in toast.find_elements(By.TAG_NAME, 'button') if 'Отправить сейчас' in b.text][0].click()
    W(d, 15).until(lambda d: sent_with('E2E отмена отправки'))
    raw = subject_of(sent_with('E2E отмена отправки')[0])[1].decode('utf-8', 'replace')
    assert 'Disposition-Notification-To: <test@rrcki.ru>' in raw, raw[:600]
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.outbox-toast'))
    ok('send now from the undo bar; read receipt requested (Disposition-Notification-To)')

    # Отложенная отправка: письмо ждёт в «Черновиках» и уходит в срок.
    compose('colleague@nrcki.ru', 'E2E отложенное', 'Уйдёт позже')
    d.find_element(By.CSS_SELECTOR, '.send-more').click()
    pop = wait(d, '.schedule-pop')
    at = time.localtime(time.time() + 100)
    set_value(d, pop.find_element(By.CSS_SELECTOR, 'input[type=datetime-local]'), time.strftime('%Y-%m-%dT%H:%M', at))
    [b for b in pop.find_elements(By.TAG_NAME, 'button') if 'Запланировать' in b.text][0].click()
    W(d, 15).until(lambda d: any('будет отправлено' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.scheduled-link'))
    drafts = [f['id'] for f in inv('list_folders')['ok'] if f['role'] == 'drafts']
    assert drafts and 'E2E отложенное' in subjects(drafts[0]), drafts
    assert not sent_with('E2E отложенное')
    ok('scheduled send: waits in Drafts, listed as scheduled')
    # Второе — на час вперёд, затем отмена: остаётся обычным черновиком.
    compose('colleague@nrcki.ru', 'E2E отложенное отменить', 'Не уйдёт')
    d.find_element(By.CSS_SELECTOR, '.send-more').click()
    pop = wait(d, '.schedule-pop')
    set_value(d, pop.find_element(By.CSS_SELECTOR, 'input[type=datetime-local]'), time.strftime('%Y-%m-%dT%H:%M', time.localtime(time.time() + 3600)))
    [b for b in pop.find_elements(By.TAG_NAME, 'button') if 'Запланировать' in b.text][0].click()
    W(d, 15).until(lambda d: '2' in d.find_element(By.CSS_SELECTOR, '.scheduled-link').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.scheduled-link').click()
    modal = wait(d, '.scheduled-modal')
    d.save_screenshot(f'{OUT}/46-scheduled.png')
    item = [i for i in modal.find_elements(By.CSS_SELECTOR, '.scheduled-item') if 'E2E отложенное отменить' in i.get_attribute('textContent')][0]
    [b for b in item.find_elements(By.TAG_NAME, 'button') if b.text == 'Отменить'][0].click()
    W(d, 15).until(lambda d: len(inv('scheduled_list')['ok']) == 1)
    assert 'E2E отложенное отменить' in subjects(drafts[0])
    d.find_element(By.CSS_SELECTOR, '.scheduled-modal button[aria-label="Закрыть"]').click()
    ok('scheduled send cancelled: stays as a plain draft')
    W(d, 180).until(lambda d: sent_with('E2E отложенное'))
    raw = subject_of(sent_with('E2E отложенное')[0])[1].decode('utf-8', 'replace')
    assert 'X-Atom-Send-At' not in raw, raw[:600]
    W(d, 20).until(lambda d: 'E2E отложенное' not in subjects(drafts[0]))
    assert len(sent_with('E2E отложенное')) == 1
    ok('scheduled mail sent on time once, removed from Drafts, service header stripped')

    # Уведомление о прочтении: запрос во входящем письме.
    msg = EmailMessage(); msg['From'] = 'Начальник <boss@nrcki.ru>'; msg['To'] = 'test@rrcki.ru'; msg['Subject'] = 'E2E MDN request'
    msg['Message-ID'] = '<mdn-request@nrcki.ru>'; msg['Disposition-Notification-To'] = 'boss@nrcki.ru'; msg.set_content('Подтвердите прочтение')
    M = imap(); M.append('INBOX', '', imaplib.Time2Internaldate(time.time()), msg.as_bytes()); M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: any('E2E MDN request' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'E2E MDN request' in r.get_attribute('textContent')][0].click()
    bar = wait(d, '.mdn-bar', 15)
    [b for b in bar.find_elements(By.TAG_NAME, 'button') if b.text == 'Отправить'][0].click()
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.mdn-bar'))
    W(d, 15).until(lambda d: any(b'disposition-notification' in open(p, 'rb').read() and b'<mdn-request@nrcki.ru>' in open(p, 'rb').read() for p in outbox()))
    assert '$MDNSent' in flags_of('INBOX', 'E2E MDN request'), flags_of('INBOX', 'E2E MDN request')
    ok('read receipt request: bar shown, MDN sent (RFC 8098), $MDNSent set so it is not asked again')

    # Трей и автозапуск.
    prefs = inv('desktop_prefs_get')['ok']
    assert prefs['tray'] is True and prefs['closeToTray'] is True, prefs
    assert inv('set_unread', {'count': 7}).get('err') is None
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    autostart_label = lambda d: [l for l in d.find_elements(By.CSS_SELECTOR, '.settings-modal label') if 'Запускать вместе с системой' in l.get_attribute('textContent')]
    auto = W(d, 15).until(autostart_label)[0].find_element(By.TAG_NAME, 'input')
    auto.click()
    desktop_file = os.path.join(os.environ['XDG_CONFIG_HOME'], 'autostart', 'ru.rrcki.mail.desktop')
    W(d, 10).until(lambda d: os.path.exists(desktop_file))
    assert '--minimized' in open(desktop_file).read()
    auto.click()
    W(d, 10).until(lambda d: not os.path.exists(desktop_file))
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
    d.execute_script("localStorage.setItem('rrcki-mail.send-prefs', JSON.stringify({undoSecs: 0}))")
    ok('tray icon with unread badge; autostart entry created and removed')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14e. категории, флажок со сроком и напоминание, цепочки, шаблоны
d = session()
try:
    wait(d, '.message-row', 30)
    d.execute_script('document.hasFocus = () => true')
    def row(subj):
        return [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if subj in r.find_element(By.CSS_SELECTOR, '.message-subject').get_attribute('textContent')]
    def flags_u(folder, subj):
        M = imap(); M.select(f'"{folder}"')
        _, ids = M.search(None, 'ALL'); found = ''
        for i in ids[0].split():
            _, data = M.fetch(i, '(FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT)])')
            if subj == str(make_header(decode_header(email.message_from_bytes(data[0][1])['Subject'] or ''))):
                found = data[0][0].decode()
        M.logout(); return found
    def menu_click(label):
        [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if label in b.get_attribute('textContent')][0].click()

    # Цепочка: письмо и ответ на него сворачиваются в одну строку.
    for subj, extra in (('E2E тред начало', {'Message-ID': '<t1@nrcki.ru>'}), ('Re: E2E тред начало', {'Message-ID': '<t2@nrcki.ru>', 'In-Reply-To': '<t1@nrcki.ru>', 'References': '<t1@nrcki.ru>'})):
        m = EmailMessage(); m['From'] = 'Коллега <colleague@nrcki.ru>'; m['To'] = 'test@rrcki.ru'; m['Subject'] = subj
        for k, v in extra.items(): m[k] = v
        m.set_content('тред'); M = imap(); M.append('INBOX', '', imaplib.Time2Internaldate(time.time()), m.as_bytes()); M.logout(); time.sleep(1.1)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: row('Re: E2E тред начало'))
    assert not row('E2E тред начало') or all('Re:' in r.find_element(By.CSS_SELECTOR, '.message-subject').get_attribute('textContent') for r in row('E2E тред начало')), 'thread not collapsed'
    head = row('Re: E2E тред начало')[0]
    assert '2' in head.find_element(By.CSS_SELECTOR, '.thread-count').get_attribute('textContent')
    head.find_element(By.CSS_SELECTOR, '.thread-count').click()
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.message-row.thread-child'))
    assert 'E2E тред начало' in d.find_element(By.CSS_SELECTOR, '.message-row.thread-child .message-subject').get_attribute('textContent')
    d.save_screenshot(f'{OUT}/47-thread.png')
    ok('threads: reply and original grouped into one row, expandable')

    # Категория: из панели письма и из контекстного меню; фильтр по категории.
    d.execute_script("arguments[0].scrollIntoView({block: 'center'})", row('Отчёт с вложением')[0])
    row('Отчёт с вложением')[0].click()
    wait(d, '.reader-content')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Категории"]').click()
    menu_click('Работа')
    W(d, 10).until(lambda d: 'Работа' in row('Отчёт с вложением')[0].get_attribute('textContent'))
    W(d, 10).until(lambda d: '$label2' in flags_u('INBOX', 'Отчёт с вложением'))
    target = row('Кодировка 1251')[0]
    d.execute_script("arguments[0].scrollIntoView({block: 'center'})", target)
    ActionChains(d).context_click(target).perform()
    menu_click('Категория «Личное»')
    W(d, 10).until(lambda d: '$label3' in (flags_u('INBOX', 'Кодировка 1251') or ''))
    Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Категория"]')).select_by_visible_text('Работа')
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.message-row')) == 1)
    d.save_screenshot(f'{OUT}/48-categories.png')
    Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Категория"]')).select_by_visible_text('Все категории')
    ok('categories: set from reader and context menu ($label2/$label3 on server), filter by category')

    # Флажок со сроком: срок уже наступил — напоминание.
    row('Отчёт с вложением')[0].click()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Срок"]').click()
    menu_click('Выбрать срок')
    pick = wait(d, 'input[aria-label="Срок"]')
    set_value(d, pick, time.strftime('%Y-%m-%dT%H:%M', time.localtime(time.time() - 120)))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.settings-modal button') if 'Поставить флажок' in b.text][0].click()
    W(d, 10).until(lambda d: 'atomdue-' in (flags_u('INBOX', 'Отчёт с вложением') or ''))
    W(d, 10).until(lambda d: row('Отчёт с вложением')[0].find_elements(By.CSS_SELECTOR, '.due-tag.overdue'))
    W(d, 40).until(lambda d: any('Напоминание по флажку' in t.get_attribute('textContent') and 'Отчёт с вложением' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    ok('flag with due date: stored on server, overdue mark, reminder shown')

    # Шаблоны: сохранить текущее письмо, вставить в новое, удалить.
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    wait(d, '.compose-modal')
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'), 'E2E шаблон')
    body = d.find_element(By.CSS_SELECTOR, '.compose-body')
    d.execute_script("arguments[0].focus(); const r = document.createRange(); r.setStart(arguments[0], 0); r.collapse(true); const s = getSelection(); s.removeAllRanges(); s.addRange(r); document.execCommand('insertText', false, 'Текст типового ответа')", body)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Шаблоны"]').click()
    pop = wait(d, '.tpl-pop')
    [b for b in pop.find_elements(By.TAG_NAME, 'button') if 'Сохранить текущее' in b.text][0].click()
    W(d, 15).until(lambda d: any('E2E шаблон' in i.get_attribute('textContent') for i in d.find_elements(By.CSS_SELECTOR, '.tpl-item')))
    assert 'E2E шаблон' in subjects('Templates')
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    wait(d, '.compose-modal')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Шаблоны"]').click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.tpl-item'))
    [i for i in d.find_elements(By.CSS_SELECTOR, '.tpl-item') if 'E2E шаблон' in i.get_attribute('textContent')][0].find_element(By.CSS_SELECTOR, '.schedule-preset').click()
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'E2E шаблон'
    W(d, 5).until(lambda d: 'Текст типового ответа' in d.find_element(By.CSS_SELECTOR, '.compose-body').get_attribute('textContent'))
    d.save_screenshot(f'{OUT}/49-template.png')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Шаблоны"]').click()
    item = [i for i in wait(d, '.tpl-pop').find_elements(By.CSS_SELECTOR, '.tpl-item') if 'E2E шаблон' in i.get_attribute('textContent')][0]
    item.find_element(By.CSS_SELECTOR, 'button[aria-label^="Удалить шаблон"]').click()
    W(d, 10).until(lambda d: 'E2E шаблон' not in subjects('Templates'))
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    ok('templates: saved to the Templates folder on the server, inserted into a new mail, deleted')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14f. перетаскивание файлов, печать, спам, отписка, «отложить»
d = session()
try:
    wait(d, '.message-row', 30)
    d.execute_script('document.hasFocus = () => true')
    def row(subj):
        return [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if subj in r.find_element(By.CSS_SELECTOR, '.message-subject').get_attribute('textContent')]
    def open_row(subj):
        r = W(d, 20).until(lambda d: row(subj))[0]
        d.execute_script("arguments[0].scrollIntoView({block: 'center'})", r); r.click()
        W(d, 15).until(lambda d: subj in d.find_element(By.CSS_SELECTOR, '.reader-content h1').get_attribute('textContent'))
    def menu_click(label):
        [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if label in b.get_attribute('textContent')][0].click()
    def add_mail(subj, headers=None, text='x'):
        m = EmailMessage(); m['From'] = 'Рассылка <news@lists.nrcki.ru>'; m['To'] = 'test@rrcki.ru'; m['Subject'] = subj
        for k, v in (headers or {}).items(): m[k] = v
        m.set_content(text); M = imap(); M.append('INBOX', '', imaplib.Time2Internaldate(time.time()), m.as_bytes()); M.logout()
    def refresh():
        d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()

    # Перетаскивание файла в окно письма — вложение.
    before = len(outbox())
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    wait(d, '.compose-modal')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), 'colleague@nrcki.ru')
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'), 'E2E перетаскивание')
    d.execute_script("""
      const dt = new DataTransfer(); dt.items.add(new File(['перетащенный файл'], 'Смета.txt', {type: 'text/plain'}));
      const t = document.querySelector('.compose-modal');
      t.dispatchEvent(new DragEvent('dragover', {bubbles: true, cancelable: true, dataTransfer: dt}));
      t.dispatchEvent(new DragEvent('drop', {bubbles: true, cancelable: true, dataTransfer: dt}));""")
    W(d, 10).until(lambda d: any('Смета.txt' in c.get_attribute('textContent') for c in d.find_elements(By.CSS_SELECTOR, '.compose-files .file-chip')))
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 20).until(lambda d: len(outbox()) > before and any(subject_of(p)[0] == 'E2E перетаскивание' for p in outbox()))
    raw = [subject_of(p)[1] for p in outbox() if subject_of(p)[0] == 'E2E перетаскивание'][0]
    assert b'filename' in raw and ('Смета.txt'.encode() in raw or b'=?utf-8?' in raw.lower()), raw[:800]
    ok('drag and drop a file into the composer attaches it')

    # Печать: документ с заголовками и текстом письма.
    open_row('Отчёт с вложением')
    d.execute_script('window.__E2E_PRINT = []')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Печать"]').click()
    printed = W(d, 5).until(lambda d: d.execute_script('return window.__E2E_PRINT'))[0]
    assert 'Отчёт с вложением' in printed and 'yushkov_do@nrcki.ru' in printed and 'Во вложении отчёт' in printed and 'Отчёт за октябрь.txt' in printed, printed[:500]
    ok('print: headers, body and attachment names in the print document')

    # «Это спам» и обратно.
    add_mail('E2E spam')
    refresh(); open_row('E2E spam')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Это спам"]').click()
    W(d, 15).until(lambda d: not row('E2E spam'))
    junk = [f['id'] for f in inv('list_folders')['ok'] if f['role'] == 'junk'][0]
    W(d, 10).until(lambda d: 'E2E spam' in subjects(junk))
    assert '$Junk' in flags_u(junk, 'E2E spam')
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Спам' in f.get_attribute('textContent')][0].click()
    open_row('E2E spam')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Не спам"]').click()
    W(d, 15).until(lambda d: 'E2E spam' in subjects('INBOX'))
    assert '$NotJunk' in flags_u('INBOX', 'E2E spam')
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Входящие' in f.get_attribute('textContent')][0].click()
    ok('spam: moved to Junk with $Junk, back to Inbox with $NotJunk')

    # Отписка: RFC 8058 одним запросом и письмом на адрес отписки.
    add_mail('E2E рассылка 1', {'List-Unsubscribe': '<mailto:unsub@lists.nrcki.ru?subject=unsubscribe-1>, <https://localhost:8443/unsub/42>', 'List-Unsubscribe-Post': 'List-Unsubscribe=One-Click'})
    add_mail('E2E рассылка 2', {'List-Unsubscribe': '<mailto:unsub@lists.nrcki.ru?subject=unsubscribe-2>'})
    refresh(); open_row('E2E рассылка 1')
    [b for b in wait(d, '.unsub-bar').find_elements(By.TAG_NAME, 'button') if 'Отписаться' in b.text][0].click()
    W(d, 15).until(lambda d: any(u['id'] == '42' and 'One-Click' in u['body'] for u in nc_mock('GET', '/test/state')['unsubscribed']))
    open_row('E2E рассылка 2')
    [b for b in wait(d, '.unsub-bar').find_elements(By.TAG_NAME, 'button') if 'Отписаться' in b.text][0].click()
    W(d, 15).until(lambda d: any(subject_of(p)[0] == 'unsubscribe-2' for p in outbox()))
    ok('unsubscribe: one-click POST (RFC 8058), otherwise e-mail to the unsubscribe address')

    # «Отложить»: письмо уходит в «Отложенные» и возвращается во «Входящие» непрочитанным.
    add_mail('E2E отложить')
    refresh(); open_row('E2E отложить')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Отложить"]').click()
    menu_click('Выбрать дату')
    set_value(d, wait(d, '.settings-modal input[type=datetime-local]'), time.strftime('%Y-%m-%dT%H:%M', time.localtime(time.time() + 60)))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.settings-modal button') if b.text == 'Отложить'][0].click()
    W(d, 15).until(lambda d: not row('E2E отложить'))
    snoozed = W(d, 10).until(lambda d: [f['id'] for f in inv('list_folders')['ok'] if 'Отложенные' in f['name']])[0]
    assert 'E2E отложить' in subjects(snoozed) and 'atomsnooze-' in flags_u(snoozed, 'E2E отложить')
    d.save_screenshot(f'{OUT}/50-snoozed.png')
    W(d, 200).until(lambda d: 'E2E отложить' in subjects('INBOX'))
    fl = flags_u('INBOX', 'E2E отложить')
    assert '\\Seen' not in fl and 'atomsnooze-' not in fl, fl
    ok('snooze: moved to Snoozed, returned to Inbox as unread on time')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14g. импорт из Thunderbird и Outlook, контакты из CSV, резервная копия и восстановление
H = os.environ['E2E_HOME']
acc = f'{H}/tb/abcd1234.default-release/Mail/Local Folders'
os.makedirs(f'{acc}/Archives.sbd', exist_ok=True)
open(f'{acc}/Inbox.msf', 'w').close()
with open(f'{acc}/Inbox', 'wb') as f:
    f.write('From - Mon Jan 15 10:00:00 2024\nX-Mozilla-Status: 0005\nX-Mozilla-Status2: 00000000\nFrom: Иванов <ivanov@example.ru>\nTo: test@rrcki.ru\nSubject: =?UTF-8?B?VEIg0L/RgNC+0YfQuNGC0LDQvdC90L7QtQ==?=\nMessage-ID: <tb-1@example.ru>\nDate: Mon, 15 Jan 2024 10:00:00 +0000\n\nПрочитанное и важное.\n>From the escaped line\n\n'.encode())
    f.write('From - Tue Jan 16 11:00:00 2024\nX-Mozilla-Status: 0000\nFrom: Сидоров <sidorov@example.ru>\nTo: test@rrcki.ru\nSubject: TB unread\nMessage-ID: <tb-2@example.ru>\nDate: Tue, 16 Jan 2024 11:00:00 +0000\n\nНе прочитано.\n\n'.encode())
    f.write(b'From - Tue Jan 16 12:00:00 2024\nX-Mozilla-Status: 0009\nSubject: TB deleted\n\ngone\n')
with open(f'{acc}/Archives.sbd/2023', 'wb') as f:
    f.write(b'From - Fri Dec 01 09:00:00 2023\nX-Mozilla-Status: 0001\nFrom: a@example.ru\nSubject: TB archive 2023\nMessage-ID: <tb-3@example.ru>\nDate: Fri, 01 Dec 2023 09:00:00 +0000\n\narchive\n')
open(f'{H}/import/one.eml', 'wb').write('From: Почта <eml@example.ru>\nSubject: EML one\nMessage-ID: <eml-1@example.ru>\nDate: Wed, 10 Jan 2024 08:00:00 +0000\n\nПервое письмо eml.\n'.encode())
open(f'{H}/import/two.eml', 'wb').write(b'From: eml@example.ru\r\nSubject: EML two\r\nMessage-ID: <eml-2@example.ru>\r\n\r\nsecond\r\n')
open(f'{H}/import/contacts.csv', 'wb').write('"Имя";"Фамилия";"Адрес эл. почты";"Телефон раб."\r\n"Анна";"Импортова";"anna.import@nrcki.ru";"+7 111"\r\n"Борис";"Импортов";"boris.import@nrcki.ru";""\r\n'.encode('cp1251'))
d = session()
try:
    wait(d, '.message-row', 30)
    def folder_id(path):
        return W(d, 20).until(lambda d: [f['id'] for f in inv('list_folders')['ok'] if f['path'] == path] or None)[0]
    def idate(folder, subj):
        M = imap(); M.select(f'"{folder}"'); _, ids = M.search(None, 'ALL'); out = ''
        for i in ids[0].split():
            _, data = M.fetch(i, '(INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT)])')
            if subj == str(make_header(decode_header(email.message_from_bytes(data[0][1])['Subject'] or ''))): out = data[0][0].decode()
        M.logout(); return out
    def tr_button(text):
        return [b for b in d.find_elements(By.CSS_SELECTOR, '.transfer-settings button') if text in b.get_attribute('textContent')][0]
    def job_done(label):
        return W(d, 120).until(lambda d: [e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, f'[aria-label="{label}"] .job-summary, [aria-label="{label}"] .login-error')] or None)[0]
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sect = wait(d, '.transfer-settings')
    d.execute_script("arguments[0].scrollIntoView({block: 'start'})", sect)

    # Thunderbird: профиль находится сам, письма с флагами и датами, удалённое пропущено.
    tr_button('Найти Thunderbird').click()
    opt = wait(d, '.transfer-source-option')
    assert 'Локальные папки' in opt.get_attribute('textContent'), opt.get_attribute('textContent')
    opt.click()
    src = wait(d, '.transfer-source').get_attribute('textContent')
    assert 'Локальные папки / Входящие' in src and 'Локальные папки / Архив / 2023' in src, src
    d.save_screenshot(f'{OUT}/51-import-source.png')
    tr_button('Начать импорт').click()
    summary = job_done('Ход импорта')
    assert 'Загружено писем: 3' in summary and 'пропущено: 1' in summary, summary
    inbox_tb = folder_id('Импорт/Локальные папки/Входящие')
    assert sorted(subjects(inbox_tb)) == ['TB unread', 'TB прочитанное'], subjects(inbox_tb)
    fl = flags_u(inbox_tb, 'TB прочитанное'); assert '\\Seen' in fl and '\\Flagged' in fl, fl
    assert '\\Seen' not in flags_u(inbox_tb, 'TB unread')
    assert '15-Jan-2024' in idate(inbox_tb, 'TB прочитанное'), idate(inbox_tb, 'TB прочитанное')
    assert subjects(folder_id('Импорт/Локальные папки/Архив/2023')) == ['TB archive 2023']
    M = imap(); M.select(f'"{inbox_tb}"'); _, ids = M.search(None, 'ALL'); _, data = M.fetch(ids[0].split()[0], '(BODY.PEEK[])'); M.logout()
    raw = data[0][1]
    assert b'X-Mozilla' not in raw and b'\r\nFrom the escaped line' in raw, raw[-200:]
    W(d, 20).until(lambda d: any('Импорт' in f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item')))
    ok('import from Thunderbird: profile found, folders kept, read/flagged/date preserved, deleted skipped')
    # Повтор — без копий.
    tr_button('Начать импорт').click()
    W(d, 60).until(lambda d: any('уже были на сервере: 3' in e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '[aria-label="Ход импорта"] .job-summary')))
    assert len(subjects(inbox_tb)) == 2
    ok('repeated import does not create duplicates')

    # Outlook .pst и .eml.
    tr_button('Файлы').click()
    src = W(d, 20).until(lambda d: 'Архив Outlook' in wait(d, '.transfer-source').get_attribute('textContent') and wait(d, '.transfer-source').get_attribute('textContent'))
    assert 'файл Outlook' in src and 'Письма' in src, src
    tr_button('Начать импорт').click()
    summary = W(d, 120).until(lambda d: [e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '[aria-label="Ход импорта"] .job-summary') if 'Загружено писем: 4' in e.get_attribute('textContent')] or None)[0]
    pst_inbox = folder_id('Импорт/Архив Outlook/Входящие')
    assert subjects(pst_inbox) == ['Отчёт из Outlook'], subjects(pst_inbox)
    assert subjects(folder_id('Импорт/Архив Outlook/Проекты/2025')) == ['Письмо в папке проекта']
    M = imap(); M.select(f'"{pst_inbox}"'); _, data = M.fetch(b'1', '(BODY.PEEK[])'); M.logout()
    msg = email.message_from_bytes(data[0][1])
    assert 'petrov@nrcki.ru' in msg['From'] and str(make_header(decode_header(msg['From']))).startswith('Петров'), msg['From']
    names = [str(make_header(decode_header(p.get_filename() or ''))) for p in msg.walk() if p.get_filename()]
    assert names == ['Смета 2025.txt'], names
    assert sorted(subjects(folder_id('Импорт/Письма'))) == ['EML one', 'EML two']
    ok('import from Outlook .pst (folders, sender, Cyrillic attachment) and .eml files')
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()

    # Контакты из CSV Outlook (Windows-1251, «;»).
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sect = wait(d, '.transfer-settings'); d.execute_script("arguments[0].scrollIntoView({block: 'start'})", sect)
    tr_button('Выбрать адресную книгу').click()
    W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, 'select[aria-label="Адресная книга для импорта"]'))
    tr_button('Загрузить контакты').click()
    summary = W(d, 60).until(lambda d: [e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '[aria-label="Ход загрузки контактов"] .job-summary')] or None)[0]
    assert 'Добавлено контактов: 2' in summary, summary
    cs = inv('list_contacts')['ok']['contacts']
    anna = [c for c in cs if 'anna.import@nrcki.ru' in c['emails']]
    assert anna and anna[0]['fullName'] == 'Импортова Анна', anna
    tr_button('Загрузить контакты').click()
    W(d, 60).until(lambda d: any('уже были в книге: 2' in e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '[aria-label="Ход загрузки контактов"] .job-summary')))
    ok('contacts from Outlook CSV (cp1251) imported once, repeat skips existing')

    # Резервная копия выбранных папок в mbox и восстановление из неё.
    d.execute_script("arguments[0].click()", [l for l in d.find_elements(By.CSS_SELECTOR, '.transfer-settings label') if 'Выбранные папки' in l.get_attribute('textContent')][0].find_element(By.TAG_NAME, 'input'))
    boxes = wait(d, '.transfer-backup-folders')
    for fid in (inbox_tb, 'INBOX'):
        d.execute_script("arguments[0].click()", boxes.find_element(By.CSS_SELECTOR, f'label[data-folder="{fid}"] input'))
    W(d, 5).until(lambda d: len([i for i in boxes.find_elements(By.CSS_SELECTOR, 'input') if i.is_selected()]) == 2)
    tr_button('Сохранить копию').click()
    summary = W(d, 120).until(lambda d: [e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '[aria-label="Ход резервного копирования"] .job-summary')] or None)[0]
    copies = glob.glob(f'{H}/backup/Atom Mail — копия */')
    assert len(copies) == 1 and 'Сохранено писем' in summary, (copies, summary)
    copy = copies[0]
    inbox_box = open(f'{copy}/Входящие.mbox', 'rb').read()
    n_inbox = len(subjects('INBOX'))
    assert inbox_box.count(b'\nFrom - ') + inbox_box.startswith(b'From - ') == n_inbox, (inbox_box.count(b'\nFrom - '), n_inbox)
    tb_box = open(f'{copy}/Импорт.sbd/Локальные папки.sbd/Входящие.mbox', 'rb').read()
    assert b'X-Mozilla-Status: 0005' in tb_box and b'X-Mozilla-Status: 0000' in tb_box, tb_box[:300]
    if os.name == 'posix':
        assert os.stat(copy).st_mode & 0o077 == 0
    d.save_screenshot(f'{OUT}/52-backup.png')
    ok(f'backup: {n_inbox} Inbox messages and nested import folder saved to mbox with flags')
    os.rmdir(f'{H}/restore'); os.rename(copy.rstrip('/'), f'{H}/restore')
    tr_button('Папка').click()
    src = W(d, 20).until(lambda d: 'Импорт / Локальные папки / Входящие' in wait(d, '.transfer-source').get_attribute('textContent') and wait(d, '.transfer-source').get_attribute('textContent'))
    root = d.find_element(By.CSS_SELECTOR, 'input[aria-label="Папка для импорта"]')
    set_value(d, root, 'Восстановлено')
    tr_button('Начать импорт').click()
    summary = W(d, 120).until(lambda d: [e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '[aria-label="Ход импорта"] .job-summary') if 'Восстановлено' in e.get_attribute('textContent')] or None)[0]
    restored = folder_id('Восстановлено/Импорт/Локальные папки/Входящие')
    fl = flags_u(restored, 'TB прочитанное'); assert '\\Seen' in fl and '\\Flagged' in fl, fl
    assert '\\Seen' not in flags_u(restored, 'TB unread')
    import re as _re
    dups = int((_re.search(r'уже были на сервере: (\d+)', summary) or [0, 0])[1])
    assert len(subjects(folder_id('Восстановлено/Входящие'))) + dups == n_inbox, (summary, n_inbox)
    ok('restore: the backup imports back with folders and flags')
    d.find_element(By.CSS_SELECTOR, '.settings-modal button[aria-label="Закрыть"]').click()
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14h. несколько учётных записей: добавление, непрочитанные, «Все входящие», переключение, отключение
d = session()
try:
    wait(d, '.message-row', 30)
    d.execute_script('document.hasFocus = () => true')
    def acc_menu():
        if not d.find_elements(By.CSS_SELECTOR, '.account-menu'):
            d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
        return wait(d, '.account-menu')
    def imap2():
        ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
        M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('second@rrcki.ru', 'Secret-456'); return M
    [b for b in acc_menu().find_elements(By.TAG_NAME, 'button') if 'Добавить учётную запись' in b.get_attribute('textContent')][0].click()
    form = wait(d, '.add-account .login-card')
    form.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').send_keys('second@rrcki.ru')
    form.find_element(By.CSS_SELECTOR, 'input[type=password]').send_keys('Secret-456')
    form.find_element(By.CSS_SELECTOR, '.login-server-toggle').click()
    host, port, smtp_host, smtp_port = form.find_elements(By.CSS_SELECTOR, '.login-server input')
    clear(host); host.send_keys('localhost'); clear(port); port.send_keys('10993')
    clear(smtp_host); smtp_host.send_keys('localhost'); clear(smtp_port); smtp_port.send_keys('465')
    d.save_screenshot(f'{OUT}/53-add-account.png')
    form.find_element(By.CSS_SELECTOR, '.login-submit').click()
    W(d, 20).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.add-account'))
    items = W(d, 10).until(lambda d: len(acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item')) == 2 and acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item'))
    assert 'second@rrcki.ru' in items[1].get_attribute('textContent') and 'текущая' in items[0].get_attribute('textContent')
    accounts_file = os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/accounts.json'
    saved = json.load(open(accounts_file))
    assert [a['username'] for a in saved] == ['test@rrcki.ru', 'second@rrcki.ru'], saved
    assert 'Secret-456' not in open(accounts_file).read() and os.stat(accounts_file).st_mode & 0o077 == 0
    ok('second account added (password checked, kept in keyring, not in accounts.json)')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
    # Новое письмо во втором ящике: число непрочитанных на кнопке учётной записи.
    M = imap2()
    for subj in ('E2E второй ящик', 'E2E второй ящик 2'):
        m = EmailMessage(); m['From'] = 'Коллега <colleague@nrcki.ru>'; m['To'] = 'second@rrcki.ru'; m['Subject'] = subj; m.set_content('письмо второго ящика')
        M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), m.as_bytes())
    M.logout()
    W(d, 60).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.account-badge') and d.find_element(By.CSS_SELECTOR, '.account-badge').get_attribute('textContent') == '2')
    ok('unread mail of the other account counted on the account button')
    # «Все входящие».
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Все входящие' in f.get_attribute('textContent')][0].click()
    rows = W(d, 30).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.unified-row') if 'E2E второй ящик 2' in r.get_attribute('textContent')] and d.find_elements(By.CSS_SELECTOR, '.unified-row'))
    owners = {r.get_attribute('data-account') for r in rows}
    assert owners == {'test@rrcki.ru@localhost', 'second@rrcki.ru@localhost'}, owners
    assert 'E2E второй ящик' in rows[0].get_attribute('textContent'), rows[0].get_attribute('textContent')
    d.save_screenshot(f'{OUT}/54-unified-inbox.png')
    ok('unified inbox: newest mail of both accounts together')
    # Письмо своей учётной записи открывается на месте, чужой — с переключением.
    mine = [r for r in rows if r.get_attribute('data-account') == 'test@rrcki.ru@localhost'][0]
    subj_mine = mine.find_element(By.CSS_SELECTOR, '.message-subject').get_attribute('textContent')
    mine.click()
    W(d, 15).until(lambda d: subj_mine in d.find_element(By.CSS_SELECTOR, '.reader-content h1').get_attribute('textContent'))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.unified-row') if 'E2E второй ящик 2' in r.get_attribute('textContent')][0].click()
    W(d, 30).until(lambda d: 'second@rrcki.ru' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent'))
    W(d, 20).until(lambda d: 'E2E второй ящик 2' in d.find_element(By.CSS_SELECTOR, '.reader-content h1').get_attribute('textContent'))
    W(d, 20).until(lambda d: any('E2E второй ящик' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    assert json.load(open(os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/settings.json'))['username'] == 'second@rrcki.ru'
    ok('opening a message of the other account switches to it and opens the message')
    # Обратно — из меню, без ввода пароля.
    [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'test@rrcki.ru' in i.get_attribute('textContent')][0].find_element(By.CSS_SELECTOR, '.account-pick').click()
    W(d, 30).until(lambda d: 'test@rrcki.ru' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent'))
    wait(d, '.message-row', 30)
    assert not any('E2E второй ящик' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row'))
    ok('switch back from the account menu without a password prompt')
    # Отключение второй учётной записи.
    item = [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'second@rrcki.ru' in i.get_attribute('textContent')][0]
    item.find_element(By.CSS_SELECTOR, 'button[aria-label="Отключить second@rrcki.ru"]').click()
    [b for b in wait(d, '.confirm-modal').find_elements(By.TAG_NAME, 'button') if b.text == 'Отключить'][0].click()
    W(d, 15).until(lambda d: len(acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item')) == 1)
    assert [a['username'] for a in json.load(open(accounts_file))] == ['test@rrcki.ru']
    import shutil
    if shutil.which('secret-tool'):
        r = subprocess.run(['secret-tool', 'search', '--all', 'service', 'ru.rrcki.mail'], capture_output=True, text=True)
        assert 'second@rrcki.ru' not in r.stdout + r.stderr, r.stdout
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
    W(d, 10).until(lambda d: not [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Все входящие' in f.get_attribute('textContent')])
    ok('second account removed: list, keyring password and unified inbox gone')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 15. перезапуск → автовход из keyring
d = session()
try:
    wait(d, '.message-row', 30); ok('auto-login from keyring after restart')
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.room-row')) == 4 and not d.find_elements(By.CSS_SELECTOR, '.nc-connect.card'))
    ok('Nextcloud still connected after restart (app password from keyring)')
    d.find_element(By.CSS_SELECTOR, '.account-switch').click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.account-menu button') if 'Выйти' in b.text][0].click()
    wait(d, '.login-card'); ok('logout -> login screen')
    W(d, 10).until(lambda d: nc_mock('GET', '/test/state')['revoked'] is True)
    cfg = [json.load(open(f)) for f in glob.glob(os.environ['XDG_CONFIG_HOME'] + '/*/settings.json')]
    assert cfg and all(not c.get('nextcloudLogin') for c in cfg), cfg
    ok('logout also revokes and forgets the Nextcloud app password')
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
# 17. вход «адрес + пароль»: серверы определяются автоматически (autoconfig)
for f in glob.glob(os.environ['XDG_CONFIG_HOME'] + '/*/settings.json'):
    os.remove(f)
d = session()
try:
    wait(d, '.login-card', 30)
    assert 'определятся автоматически' in d.find_element(By.CSS_SELECTOR, '.login-server-toggle').get_attribute('textContent')
    d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').send_keys('test@rrcki.ru')
    d.find_element(By.CSS_SELECTOR, 'input[type=password]').send_keys('Secret-123')
    d.find_element(By.CSS_SELECTOR, '.login-submit').click()
    wait(d, '.message-row', 40)
    cfg = [json.load(open(f)) for f in glob.glob(os.environ['XDG_CONFIG_HOME'] + '/*/settings.json')][0]
    assert cfg['host'] == 'localhost' and cfg['port'] == 10993 and cfg['smtpPort'] == 465, cfg
    ok('login with address and password only: servers discovered (autoconfig)')
    r = d.execute_async_script("const cb = arguments[0]; window.__TAURI_INTERNALS__.invoke('discover_servers', {email: 'someone@yandex.ru'}).then(cb, (e) => cb(String(e)))")
    assert r['imapHost'] == 'imap.yandex.ru' and r['smtpPort'] == 465 and 'пароль приложения' in r['note'], r
    ok('known providers (Yandex, Mail.ru, Gmail…) need no network lookup')
finally:
    d.quit()
print('ALL E2E PASSED')
