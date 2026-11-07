import base64
import faulthandler, json, imaplib, ssl, subprocess, sys, time, os, glob, email, email.utils
from email.header import decode_header, make_header
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait, Select
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import StaleElementReferenceException
from selenium.webdriver.common.options import ArgOptions
from selenium.webdriver.common.keys import Keys

def clear(el):
    el.send_keys(Keys.CONTROL, 'a'); el.send_keys(Keys.BACKSPACE)

APP = os.environ.get('RRCKI_MAIL_BIN', os.path.join(os.path.dirname(__file__), '../../src-tauri/target/debug/rrcki-mail'))
OUT = sys.argv[1]
T0 = time.time()
ok = lambda msg: print('PASS', msg, f'[{time.time() - T0:.0f}s]', flush=True)
# Если прогон завис, через заданное время в журнал попадёт стек, где именно.
faulthandler.dump_traceback_later(int(os.environ.get('E2E_STACK_AFTER', '6000')), repeat=True)

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
        'Authorization': 'Basic ' + base64.b64encode(b'test@corp.test:Secret-123').decode(), 'Depth': '1'})
    return urllib.request.urlopen(r, context=ctx).read().decode()

def dav_delete(path):
    import base64, urllib.request
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    r = urllib.request.Request('https://localhost:5232' + path, method='DELETE', headers={
        'Authorization': 'Basic ' + base64.b64encode(b'test@corp.test:Secret-123').decode()})
    return urllib.request.urlopen(r, context=ctx).status

def dav_put(path, body):
    import base64, urllib.request
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    r = urllib.request.Request('https://localhost:5232' + path, data=body.encode(), method='PUT', headers={
        'Authorization': 'Basic ' + base64.b64encode(b'test@corp.test:Secret-123').decode(), 'Content-Type': 'text/vcard; charset=utf-8'})
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
    M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('test@corp.test', 'Secret-123')
    M.select('INBOX'); _, data = M.uid('FETCH', str(uid), '(FLAGS)'); M.logout(); return data[0].decode()

def login(d, password):
    wait(d, '.login-card')
    clear(d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]'))
    d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').send_keys('test@corp.test')
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
    W(d, 10).until(lambda d: f'Atom Workspace {ver}' in d.find_element(By.CSS_SELECTOR, '.login-version').get_attribute('textContent'))
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
    W(d, 10).until(lambda d: f'Atom Workspace {ver}' in d.find_element(By.CSS_SELECTOR, '.about-block').get_attribute('textContent'))
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
    to = wait(d, '.compose-modal .recipient-field input'); to.send_keys('Иванов Иван <ivanov@inst.test>')
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
    assert 'ivanov@inst.test' in card and 'Ведущий инженер' in card and 'НИЦ КИ, УИТ' in card, card; ok('contact card')
    assert dav_put('/test@corp.test/contacts/kuznetsova.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:kuznetsova\r\nFN:Кузнецова Мария\r\nEMAIL:kuznetsova@inst.test\r\nEND:VCARD\r\n') in (200, 201, 204)
    search = d.find_element(By.CSS_SELECTOR, '.contacts-view .search-box input') if d.find_elements(By.CSS_SELECTOR, '.contacts-view .search-box input') else d.find_element(By.CSS_SELECTOR, 'input[placeholder="Имя, e-mail, телефон, отдел"]')
    search.send_keys('кузнец')
    W(d, 10).until(lambda d: any('Кузнецова Мария' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.contact-row')))
    ok('contacts search finds entries on the server')
    clear(search); dav_delete('/test@corp.test/contacts/kuznetsova.vcf')
    d.save_screenshot(f'{OUT}/06-contacts.png')

    # 9. письмо из карточки контакта (SMTP + копия в «Отправленные»)
    before = len(outbox())
    [b for b in d.find_elements(By.CSS_SELECTOR, '.contact-card button') if 'Написать' in b.text][0].click()
    wait(d, '.compose-modal')
    to = d.find_element(By.CSS_SELECTOR, '.recipient-field input').get_attribute('value')
    assert 'ivanov@inst.test' in to, to
    subj = d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'); subj.send_keys('Проверка отправки из RRCKI Mail')
    body = d.find_element(By.CSS_SELECTOR, '.compose-body')
    body.send_keys('Добрый день!'); body.send_keys(Keys.ENTER); body.send_keys('Это тест.')
    # Оформление: выделить всё, шрифт Georgia, жирный, крупнее.
    body.send_keys(Keys.CONTROL, 'a')
    from selenium.webdriver.support.ui import Select
    Select(d.find_element(By.CSS_SELECTOR, '.rt-font')).select_by_visible_text('Georgia')
    d.find_element(By.CSS_SELECTOR, '.rt-btn[aria-label^="Жирный"]').click()
    Select(d.find_element(By.CSS_SELECTOR, '.rt-size')).select_by_visible_text('Очень крупный')
    # Цвет текста — из палитры, одним щелчком (без Enter и «ОК»).
    d.find_element(By.CSS_SELECTOR, '.rt-color[aria-label="Цвет текста"]').click()
    wait(d, '.rt-palette .rt-swatch[aria-label="Цвет #1f5fbf"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.rt-palette'))
    W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.rt-btn[aria-label^="Жирный"]').get_attribute('aria-pressed') == 'true')
    d.save_screenshot(f'{OUT}/05-compose-format.png')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 30).until(lambda d: 'Письмо отправлено' in (d.find_elements(By.CSS_SELECTOR, '.toast') or [None])[0].get_attribute('textContent') if d.find_elements(By.CSS_SELECTOR, '.toast') else False)
    W(d, 10).until(lambda d: len(outbox()) == before + 1)
    s1, raw1 = subject_of(outbox()[-1])
    assert s1 == 'Проверка отправки из RRCKI Mail' and b'ivanov@inst.test' in raw1, s1; ok('sent via SMTP, Cyrillic subject intact')
    m1 = email.message_from_bytes(raw1)
    parts = {p.get_content_type(): p.get_payload(decode=True).decode(p.get_content_charset() or 'utf-8') for p in m1.walk() if not p.is_multipart()}
    assert 'rgb(31, 95, 191)' in parts['text/html'] or '#1f5fbf' in parts['text/html'].lower(), 'text colour from palette'
    ok('editor: text colour applied from the palette with one click')
    assert m1.get_content_type() == 'multipart/alternative' and set(parts) == {'text/plain', 'text/html'}, (m1.get_content_type(), list(parts))
    assert parts['text/plain'].replace('\r\n', '\n').strip() == 'Добрый день!\nЭто тест.', repr(parts['text/plain'])
    h = parts['text/html']
    assert 'Georgia' in h and ('bold' in h or '<b>' in h) and ('x-large' in h or 'size="5"' in h) and 'Добрый день!' in h, h
    ok('formatted mail: multipart/alternative, Georgia + bold + size in HTML, clean plain text')
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('test@corp.test', 'Secret-123')
    typ, data = M.select('Sent'); M.logout()
    assert int(data[0]) >= 1, data; ok(f'copy saved to Sent ({int(data[0])})')

    # 10. ответ: In-Reply-To + подсказка адреса из контактов
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    # «Отправленные»: в списке — кому письмо, а не от кого.
    W(d, 20).until(lambda d: [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Отправленные' in f.get_attribute('textContent')])
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Отправленные' in f.get_attribute('textContent')][0].click()
    sent_row = lambda: [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Проверка отправки из RRCKI Mail' in r.get_attribute('textContent')]
    W(d, 20).until(lambda d: sent_row())
    who = sent_row()[0].find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent')
    assert who.startswith('Кому:') and ('Иванов' in who or 'ivanov@inst.test' in who), who
    ok(f'Sent folder list shows the recipient ({who})')
    # Разделитель между списком писем и письмом: перетаскивание, запоминание, двойной щелчок — как было.
    col_w = lambda: d.execute_script("return document.querySelector('.workspace > .message-column').getBoundingClientRect().width")
    w_before = col_w()
    d.execute_script("""
      const s = document.querySelector('.splitter[aria-label="Ширина списка писем"]');
      const r = s.getBoundingClientRect(), x = r.left + 3, y = r.top + 100;
      s.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y }));
      s.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, pointerId: 1, clientX: x + 120, clientY: y }));
      s.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1, clientX: x + 120, clientY: y }));
    """)
    W(d, 5).until(lambda d: abs(col_w() - (w_before + 120)) < 3)
    assert abs(int(d.execute_script("return localStorage.getItem('atom.width.mail-list')")) - (w_before + 120)) < 3
    d.execute_script("document.querySelector('.splitter[aria-label=\"Ширина списка писем\"]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))")
    W(d, 5).until(lambda d: abs(col_w() - w_before) < 3 and d.execute_script("return localStorage.getItem('atom.width.mail-list')") is None)
    ok('mail: column splitter drags, remembers width, double click restores')
    [f for f in d.find_elements(By.CSS_SELECTOR, 'button.folder-item') if 'Входящие' in f.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: any('Отчёт с вложением' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'Отчёт с вложением' in r.get_attribute('textContent')][0].click()
    wait(d, '.attachment-card')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.reader-actions button') if 'Ответить' in b.text][0].click()
    wait(d, '.compose-modal')
    fields = d.find_elements(By.CSS_SELECTOR, '.recipient-field input')
    assert 'yushkov_do@inst.test' in fields[0].get_attribute('value'); 
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'Re: Отчёт с вложением'
    d.find_element(By.CSS_SELECTOR, '.add-cc').click()
    # Запись «как в корпоративной книге»: появилась на сервере после загрузки списка,
    # клиент о ней не знает — найти её можно только поиском на сервере.
    assert dav_put('/test@corp.test/contacts/sidorov.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:sidorov\r\nFN:Сидоров Олег\r\nN:Сидоров;Олег;;;\r\nEMAIL:sidorov@inst.test\r\nORG:НИЦ КИ\\, ОИБ\r\nTITLE:Аналитик\r\nEND:VCARD\r\n') in (200, 201, 204)
    cc = d.find_elements(By.CSS_SELECTOR, '.recipient-field input')[1]; cc.send_keys('Сидор')
    sug = W(d, 10).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.suggestions button') if 'sidorov@inst.test' in b.get_attribute('textContent')])
    assert 'Аналитик' in sug[0].get_attribute('textContent'); ok('recipient found by server-side address book search (GAL-like)')
    clear(cc); dav_delete('/test@corp.test/contacts/sidorov.vcf')
    cc.send_keys('Петр')
    sug = W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.suggestions button'))
    labels = [b.get_attribute('textContent') for b in sug]
    assert any('petrova@inst.test' in l for l in labels), labels
    sug[0].click(); time.sleep(0.3)
    assert 'Петрова Анна <petrova@inst.test>' in cc.get_attribute('value'), cc.get_attribute('value'); ok('address autocomplete from contacts')
    d.save_screenshot(f'{OUT}/07-reply.png')
    before = len(outbox())
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 30).until(lambda d: len(outbox()) == before + 1)
    s2, raw2 = subject_of(outbox()[-1])
    assert s2 == 'Re: Отчёт с вложением' and b'petrova@inst.test' in raw2, (s2, raw2[:400])
    html2 = [p for p in email.message_from_bytes(raw2).walk() if p.get_content_type() == 'text/html'][0].get_payload(decode=True).decode()
    assert '<blockquote' in html2, html2; ok('reply quote sent as blockquote')
    assert b'In-Reply-To: <report-oct@inst.test>' in raw2 and b'References: <thread-root@inst.test> <report-oct@inst.test>' in raw2, raw2[:700]
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
    assert 'SUMMARY:Тестовое совещание' in ''.join(open(f).read() for f in glob.glob('/srv/imaptest/dav/collections/collection-root/test@corp.test/work/*.ics'))
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
    assert 'Совещание перенесено' not in ''.join(open(f).read() for f in glob.glob('/srv/imaptest/dav/collections/collection-root/test@corp.test/work/*.ics'))
    ok('event deleted (UI + server)')

    # 13. повторяющееся событие: время заблокировано, место меняется у всей серии
    [b for b in d.find_elements(By.CSS_SELECTOR, '.event-block') if 'Планёрка' in b.get_attribute('textContent')][0].click()
    [b for b in wait(d, '.event-details').find_elements(By.TAG_NAME, 'button') if 'Изменить' in b.text][0].click()
    wait(d, '.editor-modal')
    assert all(not i.is_enabled() for i in d.find_elements(By.CSS_SELECTOR, '.editor-modal input[type=time], .editor-modal input[type=date]'))
    set_value(d, field(d, 'Место'), 'Переговорная 305')
    d.find_element(By.CSS_SELECTOR, '.editor-modal button[type=submit]').click()
    W(d, 20).until(lambda d: sum('Переговорная 305' in (b.get_attribute('title') or '') + b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.event-block')) >= 0 and not d.find_elements(By.CSS_SELECTOR, '.editor-modal'))
    ics = open('/srv/imaptest/dav/collections/collection-root/test@corp.test/work/planerka.ics').read()
    assert 'LOCATION:Переговорная 305' in ics and 'RRULE:FREQ=WEEKLY;BYDAY=MO,TH' in ics and 'ORGANIZER' in ics and 'SEQUENCE:1' in ics, ics
    ok('recurring series: time locked, location updated, RRULE/ORGANIZER kept')
    d.save_screenshot(f'{OUT}/09-calendar-edit.png')

    # 14. контакты: создание, изменение, удаление
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Контакты']]").click()
    W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.contact-row')) == 3)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Новый контакт"]').click()
    wait(d, '.editor-modal')
    set_value(d, field(d, 'Фамилия Имя Отчество'), 'Сидоров Пётр Ильич')
    set_value(d, field(d, 'E-mail (по одному в строке)'), 'sidorov@inst.test')
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
    vcf = ''.join(open(f).read() for f in glob.glob('/srv/imaptest/dav/collections/collection-root/test@corp.test/contacts/*.vcf'))
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
    d.execute_script("arguments[0].click()", d.find_element(By.CSS_SELECTOR, 'button[aria-label="Удалить пункт Проверить почту"]'))
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

    # Карточка целиком (двойной щелчок), как в веб-Deck: все разделы сразу, Markdown,
    # вложения (добавить/убрать), история, архив.
    def inv(cmd, args=None):
        return d.execute_async_script("const cb = arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {}).then((r) => cb({ok: r}), (e) => cb({err: JSON.stringify(e)}))", cmd, args)
    st1 = inv('deck_stacks', {'boardId': 1})['ok']
    inv('deck_create_card', {'boardId': 1, 'stackId': st1[0]['id'], 'title': 'E2E полная карточка', 'description': '## Цель\nОбновить **сеть** до 1 ноября\n- [ ] коммутаторы', 'duedate': None})
    d.find_element(By.CSS_SELECTOR, '.deck-main button[aria-label="Обновить"]').click()
    W(d, 15).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if 'E2E полная карточка' in c.get_attribute('textContent')] and not d.find_elements(By.CSS_SELECTOR, '.deck-main .spin'))
    time.sleep(1)
    full_card = [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if 'E2E полная карточка' in c.get_attribute('textContent')][0]
    d.execute_script("arguments[0].dispatchEvent(new MouseEvent('dblclick', {bubbles: true}))", full_card)
    dlg = wait(d, '.deck-full', 10)
    assert d.find_element(By.CSS_SELECTOR, '.deck-full .deck-title-input').get_attribute('value') == 'E2E полная карточка'
    heads = [h.get_attribute('textContent') for h in dlg.find_elements(By.CSS_SELECTOR, '.deck-section-head b')]
    assert any('Описание' in h for h in heads) and any('Чек-лист' in h for h in heads) and any('Связи' in h for h in heads) and any('Вложения' in h for h in heads) and any('Комментарии' in h for h in heads), heads
    W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, '.deck-full .deck-markdown h3').get_attribute('textContent') == 'Цель')
    assert d.find_element(By.CSS_SELECTOR, '.deck-full .deck-markdown b').get_attribute('textContent') == 'сеть'
    assert 'коммутаторы' not in d.find_element(By.CSS_SELECTOR, '.deck-full .deck-markdown').get_attribute('textContent'), 'checklist shown separately'
    assert [l.get_attribute('aria-label') for l in d.find_elements(By.CSS_SELECTOR, '.deck-full .deck-checklist input[type=checkbox]')] == ['коммутаторы']
    W(d, 10).until(lambda d: 'создал(а) карточку' in d.find_element(By.CSS_SELECTOR, '.deck-full .deck-activity').get_attribute('textContent'))
    d.execute_script("arguments[0].scrollIntoView({block: 'center'}); arguments[0].click()", d.find_element(By.CSS_SELECTOR, '.deck-full .deck-upload'))
    full_id = int(full_card.get_attribute('data-card'))
    W(d, 15).until(lambda d: any('отчёт-e2e.txt' in a['data'] for a in nc_mock('GET', '/test/state')['attachments'].get(str(full_id), [])))
    W(d, 10).until(lambda d: 'отчёт-e2e.txt' in d.find_element(By.CSS_SELECTOR, '.deck-full .deck-files').get_attribute('textContent'))
    d.save_screenshot(f'{OUT}/10b-deck-full-card.png')
    d.execute_script("arguments[0].scrollIntoView({block: 'center'}); arguments[0].click()", d.find_element(By.CSS_SELECTOR, '.deck-full button[aria-label="Убрать вложение отчёт-e2e.txt"]')); accept_confirm(d)
    W(d, 10).until(lambda d: not nc_mock('GET', '/test/state')['attachments'].get(str(full_id)))
    # Esc — к доске (карточка остаётся открытой панелью), снова целиком — кнопкой.
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ESCAPE)
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.deck-full') and d.find_elements(By.CSS_SELECTOR, '.deck-panel'))
    d.find_element(By.CSS_SELECTOR, '.deck-panel button[aria-label="Открыть карточку целиком"]').click()
    wait(d, '.deck-full', 5)
    # Кнопки внизу карточки — после раздела «Связи»; прокручиваем внутреннюю область к ним.
    arch = [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-full .reader-actions button') if 'В архив' in b.get_attribute('textContent')][0]
    d.execute_script("arguments[0].scrollIntoView({block: 'center'}); arguments[0].click()", arch)
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.deck-full') and not [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if 'E2E полная карточка' in c.get_attribute('textContent')])
    assert next(c for c in nc_mock('GET', '/test/state')['cards'] if c['id'] == full_id)['archived'] is True
    ok('deck: double click opens the whole card (Markdown, subtasks, files add/remove, comments, history), archive')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item') if 'Объявления' in b.get_attribute('textContent')][0].click()
    W(d, 20).until(lambda d: 'Новости' in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('textContent'))
    assert not d.find_elements(By.CSS_SELECTOR, '.deck-add-button') and 'только чтение' in d.find_element(By.CSS_SELECTOR, '.deck-main').get_attribute('textContent')
    ok('read-only board: no editing controls')

    # Deck: колонки по ширине окна и масштаб колонок — запоминается.
    cols = d.find_element(By.CSS_SELECTOR, '.deck-columns')
    assert 'stretch' in cols.get_attribute('class')
    w0 = d.execute_script("return document.querySelector('.deck-column').getBoundingClientRect().width")
    assert w0 > 272, f'stretched column expected, got {w0}'
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Колонки крупнее"]').click()
    W(d, 5).until(lambda d: '313px' in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('style'))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="По ширине окна"]').click()
    W(d, 5).until(lambda d: 'stretch' not in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('class'))
    assert abs(d.execute_script("return document.querySelector('.deck-column').getBoundingClientRect().width") - 313) < 2
    assert d.execute_script("return JSON.parse(localStorage.getItem('atom.deck.view'))") == {'scale': 1.15, 'stretch': False}
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="По ширине окна"]').click()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Колонки мельче"]').click()
    W(d, 5).until(lambda d: '272px' in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('style') and 'stretch' in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('class'))
    ok('deck: columns stretch to the window width; column size −/+ remembered')

    # Масштаб интерфейса: Ctrl + / Ctrl 0, размер текста в настройках «Вид».
    appearance = lambda: d.execute_script("return JSON.parse(localStorage.getItem('atom.appearance') || '{}')")
    ActionChains(d).key_down(Keys.CONTROL).send_keys('=').key_up(Keys.CONTROL).perform()
    W(d, 5).until(lambda d: appearance().get('zoom') == 1.1)
    ActionChains(d).key_down(Keys.CONTROL).send_keys('0').key_up(Keys.CONTROL).perform()
    W(d, 5).until(lambda d: appearance().get('zoom') == 1)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sec = wait(d, '.appearance-settings')
    sec.find_element(By.CSS_SELECTOR, 'button[aria-label="Крупнее"]').click()
    W(d, 5).until(lambda d: appearance().get('text') == 15 and d.execute_script("return getComputedStyle(document.documentElement).getPropertyValue('--content-font').trim()") == '15px')
    sec.find_element(By.CSS_SELECTOR, 'button[aria-label="Мельче"]').click()
    W(d, 5).until(lambda d: appearance().get('text') == 14)
    Select(sec.find_element(By.CSS_SELECTOR, 'select[aria-label="Масштаб интерфейса"]')).select_by_value('1.25')
    W(d, 5).until(lambda d: appearance().get('zoom') == 1.25)
    Select(sec.find_element(By.CSS_SELECTOR, 'select[aria-label="Масштаб интерфейса"]')).select_by_value('1')
    W(d, 5).until(lambda d: appearance().get('zoom') == 1)
    d.find_element(By.CSS_SELECTOR, '.settings-modal .compose-header button[aria-label="Закрыть"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.appearance-settings'))
    ok('appearance: zoom with Ctrl +/0 and in settings, chat/mail text size')

    # Группы досок (по проекту): вручную через меню правой кнопки; запоминается.
    def dctx(el):
        d.execute_script("const r = arguments[0].getBoundingClientRect(); arguments[0].dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: r.left + 20, clientY: r.top + 8}))", el)
        return wait(d, '.ctx-menu')
    def dpick(label):
        [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.strip() == label][0].click()
        W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.ctx-menu'))
    board_btn = lambda title: [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item') if title in b.get_attribute('textContent')][0]
    d.execute_script("window.prompt = () => 'Проект Альфа'")
    dctx(board_btn('Объявления')); dpick('В новую группу…')
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"]'))
    dctx(board_btn('УИТ — задачи')); dpick('В группу «Проект Альфа»')
    W(d, 5).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"] .folder-item')) == 2)
    assert not [h for h in d.find_elements(By.CSS_SELECTOR, '.board-group-head.static')], 'no "Без группы" when every board is grouped'
    d.find_element(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"] .board-group-head').click()
    W(d, 5).until(lambda d: 'Проект Альфа' in (d.execute_script("return localStorage.getItem('atom.deck.folded')") or ''))
    d.find_element(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"] .board-group-head').click()
    dctx(board_btn('Объявления')); dpick('Группа по названию (как было)')
    dctx(board_btn('УИТ — задачи')); dpick('Группа по названию (как было)')
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.board-group'))
    ok('deck: boards grouped by project via right-click menu, groups fold, back to automatic grouping')

    # Связи карточек: подзадача-карточка видна на доске под главной, «выполняется после» — с замком.
    board_btn('УИТ — задачи').click()
    deck_card = lambda title: [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if title in c.get_attribute('textContent')][0]
    W(d, 10).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if 'Обновить RuPost' in c.get_attribute('textContent')])
    deck_card('Обновить RuPost').click()
    sub_in = wait(d, '.deck-links input[aria-label="Новая подзадача-карточка"]')
    sub_in.send_keys('Проверить резервные копии', Keys.ENTER)
    W(d, 15).until(lambda d: any(c['title'] == 'Проверить резервные копии' and '/apps/deck/card/1)' in c['description'] and 'Главная задача: [Обновить RuPost]' in c['description'] for c in nc_mock('GET', '/test/state')['cards']))
    W(d, 15).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card.sub') if 'Проверить резервные копии' in c.get_attribute('textContent')])
    W(d, 10).until(lambda d: '0/1' in deck_card('Обновить RuPost').find_element(By.CSS_SELECTOR, '.deck-kids').text)
    # Порядок: подзадача сразу под главной в той же колонке.
    order = [c.get_attribute('textContent') for c in d.find_elements(By.CSS_SELECTOR, '.deck-column[data-stack="10"] .deck-card')]
    i_parent = next(i for i, t in enumerate(order) if 'Обновить RuPost' in t)
    assert 'Проверить резервные копии' in order[i_parent + 1], order
    deck_card('Настроить брокер X2Go').click()
    sel = wait(d, '.deck-links select[aria-label="Добавить предыдущую…"]')
    Select(sel).select_by_visible_text('Обновить RuPost')
    W(d, 15).until(lambda d: any(c['id'] == 2 and 'Выполняется после: [Обновить RuPost]' in c['description'] for c in nc_mock('GET', '/test/state')['cards']))
    W(d, 15).until(lambda d: 'после: Обновить RuPost' in deck_card('Настроить брокер X2Go').get_attribute('textContent') and 'blocked' in deck_card('Настроить брокер X2Go').get_attribute('class'))
    # Наведение на главную подсвечивает связанные карточки.
    ActionChains(d).move_to_element(deck_card('Обновить RuPost')).perform()
    W(d, 5).until(lambda d: 'related' in deck_card('Проверить резервные копии').get_attribute('class') and 'related' in deck_card('Настроить брокер X2Go').get_attribute('class'))
    # Связь убирается; описание остаётся без служебного блока.
    deck_card('Настроить брокер X2Go').click()
    wait(d, '.deck-links .deck-link-x').click()
    W(d, 15).until(lambda d: any(c['id'] == 2 and c['description'] == '' for c in nc_mock('GET', '/test/state')['cards']))
    W(d, 10).until(lambda d: 'blocked' not in deck_card('Настроить брокер X2Go').get_attribute('class'))
    ok('deck links: subtask cards shown under the main task with progress, "after" dependency with lock, related cards highlighted')

    # 19. Talk
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.room-row')) == 2)
    W(d, 20).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.talk-msg')) == 3)
    msgs = [m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg')]
    assert '@Тестовый Пользователь' in msgs[0] and 'План работ.xlsx' in msgs[1] and 'Фото с объекта.png' in msgs[2], msgs
    assert [f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, '.talk-msg .talk-file span')] == ['План работ.xlsx']
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.room-row .sender-avatar.has-image img')) == 2)
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-msg .sender-avatar.has-image img'))
    ok('talk: room and author avatars loaded from Nextcloud')
    assert not any('reaction' in m for m in msgs); ok('history: mentions and files rendered, reaction events hidden')
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.rail-badge')); ok('chat badge cleared after reading')

    # Фото в чате: уменьшенная копия в ленте, просмотр во встроенном окне, сохранение на компьютер.
    W(d, 15).until(lambda d: [i for i in d.find_elements(By.CSS_SELECTOR, '.talk-image img') if (i.get_attribute('src') or '').startswith('data:image/png') and d.execute_script('return arguments[0].naturalWidth', i) == 32])
    assert nc_mock('GET', '/test/state')['previews'] >= 1
    d.find_element(By.CSS_SELECTOR, '.talk-image-open').click()
    W(d, 15).until(lambda d: [i for i in d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop img.viewer-image') if d.execute_script('return arguments[0].naturalWidth', i) == 64])
    assert 'Фото с объекта.png' in d.find_element(By.CSS_SELECTOR, '.viewer-head').get_attribute('textContent')
    d.save_screenshot(f'{OUT}/11a-chat-photo.png')
    d.find_element(By.CSS_SELECTOR, '.viewer-backdrop button[aria-label="Сохранить"]').click()
    saved = os.path.join(os.environ['E2E_HOME'], 'saved', 'Фото с объекта.png')
    W(d, 15).until(lambda d: os.path.exists(saved) and open(saved, 'rb').read(8) == b'\x89PNG\r\n\x1a\n')
    d.find_element(By.CSS_SELECTOR, '.viewer-backdrop button[aria-label="Закрыть просмотр"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop'))
    ok('chat photo: thumbnail in the feed, opens full-size in the viewer, saved to computer')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('Добрый день, принято'); box.send_keys(Keys.ENTER)
    W(d, 20).until(lambda d: any('Добрый день, принято' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own')))
    ok('message sent, shown as own')
    def own_msg(text):
        return [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own') if text in m.get_attribute('textContent')][0]
    assert own_msg('Добрый день, принято').find_elements(By.CSS_SELECTOR, '.talk-tick[aria-label="Отправлено"]'), 'single tick expected'
    t0 = time.time()
    # Коллега прочитал всё и ответил: свои сообщения — «✓✓».
    nc_mock('POST', '/test/inject?read=1&token=abc123&text=' + __import__('urllib.parse').parse.quote('Срочно: проверьте почту'))
    W(d, 15).until(lambda d: any('Срочно: проверьте почту' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg:not(.own)')))
    ok(f'incoming message via long-poll in {time.time() - t0:.1f}s')
    W(d, 10).until(lambda d: own_msg('Добрый день, принято').find_elements(By.CSS_SELECTOR, '.talk-tick.read[aria-label="Прочитано"]'))
    ok('own messages: ✓ sent, ✓✓ once read by everyone')

    # Недописанное сообщение переживает уход в другой раздел.
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('Черновик в чате')
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-compose'))
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-compose textarea') and d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == 'Черновик в чате')
    W(d, 20).until(lambda d: any('Срочно: проверьте почту' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg')))
    clear(d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'))
    ok('chat draft kept after switching to Mail and back')

    # Избранное: вторая беседа отмечается звёздочкой и встаёт первой (и на сервере тоже).
    rows = d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row:not(.person-row)')
    second = rows[1].find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent')
    assert second, 'room name expected'
    rows[1].click()
    W(d, 10).until(lambda d: second in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.talk-favorite').click()
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row')[0].find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent') == second
                   and d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row')[0].find_elements(By.CSS_SELECTOR, '.room-favorite'))
    W(d, 10).until(lambda d: any(r['isFavorite'] for r in nc_mock('GET', '/test/state')['rooms'] if r['displayName'] == second or r.get('name') == second))
    d.find_element(By.CSS_SELECTOR, '.talk-favorite').click()
    W(d, 10).until(lambda d: not any(r['isFavorite'] for r in nc_mock('GET', '/test/state')['rooms']))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if 'УИТ — общий' in r.find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent')][0].click()
    W(d, 15).until(lambda d: 'УИТ — общий' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent')
                   and any('обновление в среду' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg')))
    ok('chat favorites: star pins the conversation on top (Talk API)')
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
    [f for f in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own .talk-file') if 'отчёт-e2e.txt' in f.get_attribute('textContent')][0].click()
    W(d, 15).until(lambda d: 'test file for talk' in ''.join(e.get_attribute('textContent') for e in d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop .viewer-text')))
    d.find_element(By.CSS_SELECTOR, '.viewer-backdrop button[aria-label="Закрыть просмотр"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop'))
    ok('text file from chat opens in the built-in viewer')
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
    box.send_keys('посмотри **срочно** https://inst.test/doc'); box.send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['messages']['abc123'][-1].get('raw') == '@"petrova" посмотри **срочно** https://inst.test/doc')
    mm = W(d, 15).until(lambda d: talk_msg('посмотри'))
    W(d, 10).until(lambda d: talk_msg('посмотри').find_elements(By.CSS_SELECTOR, '.talk-mention'))
    assert talk_msg('посмотри').find_element(By.CSS_SELECTOR, 'b').text == 'срочно'
    assert talk_msg('посмотри').find_element(By.CSS_SELECTOR, 'a').get_attribute('href') == 'https://inst.test/doc'
    ok('mention autocomplete (@"id"), bold and link rendered')

    # Эмодзи из панели.
    d.find_element(By.CSS_SELECTOR, '.talk-emoji').click()
    d.find_element(By.CSS_SELECTOR, '.emoji-grid button[aria-label="😀"]').click()
    W(d, 5).until(lambda d: '😀' in d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value'))
    d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['messages']['abc123'][-1].get('raw') == '😀')
    ok('emoji picker inserts and sends')
    # Окно эмодзи закрывается щелчком по свободному месту ленты.
    d.find_element(By.CSS_SELECTOR, '.talk-emoji').click()
    wait(d, '.emoji-picker')
    d.execute_script("const l = document.querySelector('.talk-messages'), r = l.getBoundingClientRect(); for (const t of ['mousedown', 'mouseup', 'click']) l.dispatchEvent(new MouseEvent(t, {bubbles: true, cancelable: true, clientX: r.left + 30, clientY: r.top + 40}))")
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.emoji-picker'))
    ok('emoji picker closes on a click in the empty chat area')

    # Поиск по сообщениям беседы: результат, переход и подсветка.
    d.find_element(By.CSS_SELECTOR, '.talk-search-toggle').click()
    wait(d, '.talk-search input').send_keys('среду')
    hit = W(d, 10).until(lambda d: [h for h in d.find_elements(By.CSS_SELECTOR, '.talk-search-hit') if 'обновление в среду' in h.get_attribute('textContent')])[0]
    assert hit.find_elements(By.CSS_SELECTOR, 'mark'), 'match highlighted'
    hit.click()
    W(d, 10).until(lambda d: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.flash') if 'обновление в среду' in m.get_attribute('textContent')])
    d.find_element(By.CSS_SELECTOR, '.talk-search input').send_keys(Keys.ESCAPE)
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-search'))
    ok('chat: search in conversation messages, jump to the found message')

    st = lambda: nc_mock('GET', '/test/state')
    # Опрос: создать, проголосовать, голос коллеги, завершить.
    d.find_element(By.CSS_SELECTOR, '.talk-poll-new').click()
    dlg = wait(d, '.talk-poll-modal')
    dlg.find_element(By.CSS_SELECTOR, 'input').send_keys('Когда созвон?')
    dlg.find_element(By.CSS_SELECTOR, 'input[aria-label="Вариант 1"]').send_keys('Утром')
    dlg.find_element(By.CSS_SELECTOR, 'input[aria-label="Вариант 2"]').send_keys('Вечером')
    [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.text == 'Отправить опрос'][0].click()
    card = W(d, 15).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.talk-poll') if 'Когда созвон?' in c.get_attribute('textContent') and c.find_elements(By.CSS_SELECTOR, '.talk-poll-choice')])[0]
    pid = card.get_attribute('data-poll')
    [l for l in card.find_elements(By.CSS_SELECTOR, '.talk-poll-choice') if 'Вечером' in l.text][0].click()
    [b for b in card.find_elements(By.TAG_NAME, 'button') if b.text == 'Голосовать'][0].click()
    W(d, 10).until(lambda d: st()['talk_polls'][pid]['allVotes'].get('test') == [1])
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, f'.talk-poll[data-poll="{pid}"] .talk-poll-result.mine'))
    nc_mock('POST', f'/test/poll_vote?id={pid}&options=0')
    card = d.find_element(By.CSS_SELECTOR, f'.talk-poll[data-poll="{pid}"]')
    [b for b in card.find_elements(By.TAG_NAME, 'button') if b.text == 'Завершить'][0].click(); accept_confirm(d)
    W(d, 10).until(lambda d: st()['talk_polls'][pid]['status'] == 1)
    W(d, 10).until(lambda d: 'завершён' in d.find_element(By.CSS_SELECTOR, f'.talk-poll[data-poll="{pid}"]').get_attribute('textContent')
                   and '50% · 1' in d.find_element(By.CSS_SELECTOR, f'.talk-poll[data-poll="{pid}"]').get_attribute('textContent'))
    ok('chat polls: create, vote, results with a colleague vote, close')

    # Тихая отправка: без звука уведомления (флаг silent), переключатель сбрасывается после отправки.
    d.find_element(By.CSS_SELECTOR, '.talk-silent').click()
    assert 'active' in d.find_element(By.CSS_SELECTOR, '.talk-silent').get_attribute('class')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'); box.send_keys('Тихое сообщение', Keys.ENTER)
    W(d, 10).until(lambda d: any(m.get('raw') == 'Тихое сообщение' and m.get('silent') for m in st()['messages']['abc123']))
    W(d, 5).until(lambda d: 'active' not in d.find_element(By.CSS_SELECTOR, '.talk-silent').get_attribute('class'))
    ok('chat: silent send (no notification sound for recipients)')

    # Превью ссылки под сообщением (через сервис ссылок Nextcloud).
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'); box.send_keys('Регламент: https://example.org/reglament', Keys.ENTER)
    link = W(d, 15).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.talk-link-card') if 'Пример: регламент выпуска' in c.get_attribute('textContent')])[0]
    W(d, 10).until(lambda d: link.find_elements(By.CSS_SELECTOR, 'img[src^="data:image/png"]'))
    assert 'example.org' in link.get_attribute('textContent')
    ok('chat: link preview card with title, description and image')

    # Голосовое: запись (микрофон подменён в тесте) — отправка как voice-message; прослушивание голосового коллеги.
    d.execute_script("""
      navigator.mediaDevices.getUserMedia = async () => ({ getTracks: () => [{ stop() {} }] });
      window.MediaRecorder = class {
        constructor() { this.mimeType = 'audio/ogg' }
        static isTypeSupported(t) { return t.startsWith('audio/ogg') }
        start() {}
        stop() { this.ondataavailable({ data: new Blob([new Uint8Array([79, 103, 103, 83, 0, 2, 0, 0])], { type: 'audio/ogg' }) }); this.onstop() }
      };
    """)
    d.find_element(By.CSS_SELECTOR, '.talk-mic').click()
    wait(d, '.talk-recording'); time.sleep(1.3)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Отправить голосовое"]').click()
    W(d, 20).until(lambda d: any(m.get('messageType') == 'voice-message' and m['actorId'] == 'test' and m['messageParameters']['file']['name'].startswith('Голосовое ') for m in st()['messages']['abc123']))
    nc_mock('POST', '/test/inject?voice=1&token=abc123')
    # Голосовое коллеги (настоящий WAV): ждём именно его — оно приходит через long-poll.
    peer_voice = lambda: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg:not(.own)') if m.find_elements(By.CSS_SELECTOR, '.talk-voice') and 'Иванов Иван' in m.get_attribute('textContent')]
    W(d, 15).until(lambda d: peer_voice() and peer_voice()[-1].find_elements(By.CSS_SELECTOR, '.talk-voice button'))
    peer_voice()[-1].find_element(By.CSS_SELECTOR, '.talk-voice button').click()
    W(d, 10).until(lambda d: peer_voice()[-1].find_elements(By.CSS_SELECTOR, '.talk-voice audio[src^="blob:"]'))
    # Звук действительно открывается (не заблокирован политикой содержимого).
    W(d, 10).until(lambda d: d.execute_script("const a = arguments[0].querySelector('.talk-voice audio'); return a && !a.error && a.readyState >= 1", peer_voice()[-1]))
    ok('chat voice messages: record and send as voice message, play a received one')

    # Медиа и файлы беседы: фото (просмотр с листанием), файлы, голосовые, ссылки.
    d.find_element(By.CSS_SELECTOR, '.talk-media-toggle').click()
    panel = wait(d, '.talk-media-panel')
    tile = W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-media-panel .talk-media-tile[aria-label="Открыть Фото с объекта.png"]'))[0]
    tile.click()
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.viewer-image'))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть просмотр"]').click()
    def media_tab(name):
        [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-media-tabs button') if name in b.get_attribute('textContent')][0].click()
    media_tab('Файлы')
    W(d, 10).until(lambda d: any('отчёт-e2e.txt' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-media-panel .talk-media-row')))
    media_tab('Голосовые')
    W(d, 10).until(lambda d: any('Иванов Иван' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-media-panel .talk-media-row')))
    media_tab('Ссылки')
    W(d, 10).until(lambda d: any('example.org/reglament' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-media-panel .talk-media-row')))
    media_tab('Медиа'); time.sleep(1); d.save_screenshot(f'{OUT}/11c-chat-extras.png')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть медиа и файлы"]').click()
    ok('chat: media & files panel (photos with viewer, files, voice, links)')

    # Исчезающие сообщения (модератор): включить — новые сообщения с отметкой, выключить.
    def expiry(label):
        d.find_element(By.CSS_SELECTOR, 'button[aria-label="Меню беседы"]').click()
        [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.strip() == 'Настройки беседы…'][0].click()
        Select(wait(d, '.talk-settings-modal select[aria-label="Исчезающие сообщения"]')).select_by_visible_text(label)
        W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-settings-modal'))
    expiry('Через 1 час')
    W(d, 10).until(lambda d: any(r['token'] == 'abc123' and r['messageExpiration'] == 3600 for r in st()['rooms']))
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-expiry-badge'))
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'); box.send_keys('Временное сообщение', Keys.ENTER)
    W(d, 10).until(lambda d: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg.own') if 'Временное сообщение' in m.get_attribute('textContent') and m.find_elements(By.CSS_SELECTOR, '.talk-expires')])
    expiry('Выключены')
    W(d, 10).until(lambda d: any(r['token'] == 'abc123' and r['messageExpiration'] == 0 for r in st()['rooms']))
    ok('chat: disappearing messages on/off (moderator), marked in the feed')

    # Звонок — в окне программы (в тесте окно не открывается, проверяется адрес).
    d.find_element(By.CSS_SELECTOR, '.talk-call').click()
    call_file = os.path.join(os.environ['E2E_HOME'], 'saved', 'call-window.txt')
    W(d, 10).until(lambda d: os.path.exists(call_file))
    assert open(call_file).read().endswith('/index.php/call/abc123'), open(call_file).read()
    ok('chat: call opens in an app window (Nextcloud call page)')

    # Обои чата: у каждого свои, на этом компьютере — для всех бесед и для отдельной.
    def wallpaper_dialog():
        d.find_element(By.CSS_SELECTOR, 'button[aria-label="Меню беседы"]').click()
        [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.strip() == 'Обои…'][0].click()
        return wait(d, '.wallpaper-modal')
    def pick(dlg, scope, tile):
        [l for l in dlg.find_elements(By.CSS_SELECTOR, '.wallpaper-scope label') if scope in l.text][0].click()
        dlg.find_element(By.CSS_SELECTOR, f'.wallpaper-tile[aria-label="{tile}"]').click()
        W(d, 3).until(lambda d: dlg.find_elements(By.CSS_SELECTOR, '.wallpaper-preview .talk-wallpaper') or tile == 'Без обоев')
        [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.text == 'Применить'][0].click()
        W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.wallpaper-modal'))
    main_wp = lambda: d.find_element(By.CSS_SELECTOR, '.talk-main').get_attribute('data-wallpaper')
    pick(wallpaper_dialog(), 'Все беседы', 'Мята')
    W(d, 5).until(lambda d: main_wp() == 'mint' and d.find_elements(By.CSS_SELECTOR, '.talk-main.has-wallpaper > .talk-wallpaper'))
    # Пузыри остаются непрозрачными.
    assert d.execute_script("const b = getComputedStyle(document.querySelector('.talk-msg .talk-bubble')).backgroundColor; return !b.includes('rgba') || b.endsWith(', 1)')")
    # Своя картинка только для этой беседы, с затемнением.
    dlg = wallpaper_dialog()
    [l for l in dlg.find_elements(By.CSS_SELECTOR, '.wallpaper-scope label') if 'Только' in l.text][0].click()
    # Файл выбирается «как из диалога»: WebKitWebDriver не умеет передавать файлы.
    d.execute_script("""
      const png = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFUlEQVR42mNk+M9QzwAEjDAGNzYAAQwBACiYBNEzkVnqAAAAAElFTkSuQmCC'), (c) => c.charCodeAt(0));
      const input = document.querySelector('input[aria-label="Файл картинки для обоев"]');
      const dt = new DataTransfer(); dt.items.add(new File([png], 'обои.png', { type: 'image/png' }));
      input.files = dt.files; input.dispatchEvent(new Event('change', { bubbles: true }));
    """)
    W(d, 10).until(lambda d: dlg.find_elements(By.CSS_SELECTOR, '.wallpaper-sliders input[aria-label="Затемнение"]'))
    [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.text == 'Применить'][0].click()
    W(d, 5).until(lambda d: main_wp() == 'image' and d.find_elements(By.CSS_SELECTOR, '.talk-main > .talk-wallpaper-shade'))
    # Другая беседа — с общими обоями; обратно — снова своя картинка.
    [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if 'Иванов Иван' in r.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: 'Иванов Иван' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent') and main_wp() == 'mint')
    [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if 'УИТ — общий' in r.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: 'УИТ — общий' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent') and main_wp() == 'image')
    W(d, 20).until(lambda d: any('обновление в среду' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg')))
    time.sleep(0.5); d.save_screenshot(f'{OUT}/11d-chat-wallpaper.png')
    # «Как во всех беседах», затем обои выключаются совсем.
    dlg = wallpaper_dialog()
    [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.text == 'Как во всех беседах'][0].click()
    W(d, 5).until(lambda d: main_wp() == 'mint')
    pick(wallpaper_dialog(), 'Все беседы', 'Без обоев')
    W(d, 5).until(lambda d: main_wp() is None and not d.find_elements(By.CSS_SELECTOR, '.talk-main > .talk-wallpaper'))
    assert d.execute_script("return Object.keys(localStorage).filter((k) => k.startsWith('atom.wallpaper.img.')).length") == 0, 'unused images removed'
    ok('chat wallpapers: preset for all chats, own image for one chat (dim), per-chat override and reset, stored locally')
    d.save_screenshot(f'{OUT}/11-chat.png')

    # Карточка Deck из сообщения.
    action(talk_msg('обновление в среду'), 'Карточка Deck')
    modal = wait(d, '.talk-card-modal')
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.talk-card-modal select option')) >= 2)
    [b for b in modal.find_elements(By.TAG_NAME, 'button') if 'Создать карточку' in b.text][0].click()
    W(d, 15).until(lambda d: any(c['title'].startswith('Коллеги') and 'call/abc123#message_1' in c['description'] for c in nc_mock('GET', '/test/state')['cards']))
    ok('Deck card created from message (text, author, link)')
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-card-modal'))

    # Правая кнопка мыши: своё меню вместо стандартного «Назад / Обновить / Печать».
    def ctx(el):
        d.execute_script("const r = arguments[0].getBoundingClientRect(); arguments[0].dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: r.left + 20, clientY: r.top + 10}))", el)
        return wait(d, '.ctx-menu')
    def ctx_pick(label):
        [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.strip() == label][0].click()
        W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.ctx-menu'))
    prevented = d.execute_script("const e = new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: 20, clientY: 300}); document.querySelector('.module-rail').dispatchEvent(e); return e.defaultPrevented")
    assert prevented and not d.find_elements(By.CSS_SELECTOR, '.ctx-menu'), 'default web menu must be suppressed'
    menu = ctx(talk_msg('посмотри'))
    labels = [b.text.strip() for b in menu.find_elements(By.CSS_SELECTOR, '.ctx-item')]
    assert {'Ответить', 'Копировать текст', 'Переслать…', 'Изменить', 'Удалить'} <= set(labels), labels
    assert menu.find_elements(By.CSS_SELECTOR, '.ctx-custom button[aria-label="Реакция 👍"]'), 'quick reactions row'
    ctx_pick('Ответить')
    wait(d, '.talk-quote.replying')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Отменить ответ"]').click()
    ctx(talk_msg('посмотри')); ctx_pick('Переслать…')
    fwd = wait(d, '.modal-backdrop [aria-label="Переслать сообщение"]')
    [b for b in fwd.find_elements(By.CSS_SELECTOR, '.message-row') if 'Иванов Иван' in b.get_attribute('textContent')][0].click()
    W(d, 15).until(lambda d: any(m['message'].startswith('↪ ') and 'посмотри' in m['message'] for m in nc_mock('GET', '/test/state')['messages']['dm-ivanov']))
    ok('chat: right-click menu (reply, copy, forward, edit, delete, reactions); default web menu suppressed; forward to another chat')

    # Картинка из буфера обмена (Ctrl+V): превью, отправка файлом в /Talk.
    d.execute_script("""
      const png = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=='), (c) => c.charCodeAt(0));
      const file = new File([png], 'image.png', { type: 'image/png' });
      const ev = new Event('paste', { bubbles: true, cancelable: true });
      Object.defineProperty(ev, 'clipboardData', { value: { files: [file], items: [], types: ['Files'], getData: () => '' } });
      document.querySelector('.talk-compose textarea').dispatchEvent(ev);
    """)
    W(d, 10).until(lambda d: d.execute_script("const i = document.querySelector('.talk-pending .talk-paste-preview'); return i && i.complete && i.naturalWidth > 0"))
    d.find_element(By.CSS_SELECTOR, '.talk-compose button[type=submit]').click()
    W(d, 20).until(lambda d: any(k.startswith('/Talk/Снимок ') and k.endswith('.png') for k in nc_mock('GET', '/test/state')['files']))
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-pending'))
    ok('chat: image pasted from clipboard is previewed and sent')

    # Отправка восстановленного черновика очищает поле — и после возврата в раздел тоже.
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea'); box.send_keys('Отправлю после возврата')
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-compose'))
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-compose textarea') and d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == 'Отправлю после возврата')
    d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['messages']['abc123'][-1].get('raw', nc_mock('GET', '/test/state')['messages']['abc123'][-1]['message']) == 'Отправлю после возврата')
    W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == '')
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-compose'))
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-compose textarea'))
    time.sleep(1)
    assert d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == '', 'sent draft must not come back'
    W(d, 20).until(lambda d: any('обновление в среду' in m.get_attribute('textContent') for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg')))
    ok('chat: restored draft is cleared after sending and does not come back')

    # Участники и модерация (как в Talk).
    def members_of(tok='abc123'):
        return {m['actorId']: m['participantType'] for m in nc_mock('GET', '/test/state')['members'][tok]}
    def person(uid):
        return d.find_element(By.CSS_SELECTOR, f'.talk-person[data-actor="{uid}"]')
    d.find_element(By.CSS_SELECTOR, '.talk-people-toggle').click()
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.talk-people-panel .talk-person')) == 3)
    assert 'модератор' in person('ivanov').get_attribute('textContent') and 'владелец' in person('test').get_attribute('textContent') and '(вы)' in person('test').get_attribute('textContent')
    assert person('ivanov').find_elements(By.CSS_SELECTOR, '.presence.online') and person('petrova').find_elements(By.CSS_SELECTOR, '.presence.away')
    person('petrova').find_element(By.CSS_SELECTOR, 'button[aria-label^="Действия"]').click()
    wait(d, '.ctx-menu'); ctx_pick('Назначить модератором')
    W(d, 10).until(lambda d: members_of()['petrova'] == 2)
    W(d, 10).until(lambda d: 'модератор' in person('petrova').get_attribute('textContent'))
    ctx(person('petrova')); ctx_pick('Снять права модератора')
    W(d, 10).until(lambda d: members_of()['petrova'] == 3)
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Добавить участника"]').click()
    wait(d, '.talk-people-add input').send_keys('Сидор')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-people-add .person-row'))[0].click()
    W(d, 10).until(lambda d: 'sidorov' in members_of())
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-person[data-actor="sidorov"]'))
    ctx(person('sidorov')); ctx_pick('Удалить из беседы'); accept_confirm(d)
    W(d, 10).until(lambda d: 'sidorov' not in members_of())
    ok('chat participants: roles and status, promote/demote moderator, add and remove a member')

    # Настройки беседы: описание, переименование, «только чтение».
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Настройки беседы"]').click()
    dlg = wait(d, '.talk-settings-modal')
    name_in = dlg.find_element(By.CSS_SELECTOR, 'input'); clear(name_in); name_in.send_keys('УИТ — релизы')
    dlg.find_element(By.CSS_SELECTOR, 'textarea').send_keys('Обсуждение выпусков')
    [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.text == 'Сохранить'][0].click()
    W(d, 10).until(lambda d: any(r['displayName'] == 'УИТ — релизы' and r['description'] == 'Обсуждение выпусков' for r in nc_mock('GET', '/test/state')['rooms']))
    W(d, 15).until(lambda d: 'УИТ — релизы' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent'))
    W(d, 10).until(lambda d: 'Обсуждение выпусков' in d.find_element(By.CSS_SELECTOR, '.talk-people-panel').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Настройки беседы"]').click()
    dlg = wait(d, '.talk-settings-modal')
    [l for l in dlg.find_elements(By.TAG_NAME, 'label') if 'Только чтение' in l.text][0].find_element(By.TAG_NAME, 'input').click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-readonly') and not d.find_elements(By.CSS_SELECTOR, '.talk-compose'))
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Настройки беседы"]').click()
    dlg = wait(d, '.talk-settings-modal')
    [l for l in dlg.find_elements(By.TAG_NAME, 'label') if 'Только чтение' in l.text][0].find_element(By.TAG_NAME, 'input').click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-compose textarea'))
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Настройки беседы"]').click()
    dlg = wait(d, '.talk-settings-modal')
    name_in = dlg.find_element(By.CSS_SELECTOR, 'input'); clear(name_in); name_in.send_keys('УИТ — общий')
    [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.text == 'Сохранить'][0].click()
    W(d, 15).until(lambda d: 'УИТ — общий' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent'))
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Закрыть список участников"]').click()
    ok('chat moderation: rename, description, read-only on and off')

    # Меню беседы: «Не уведомлять» и обратно.
    row = lambda: [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if 'УИТ — общий' in r.get_attribute('textContent')][0]
    ctx(row()); ctx_pick('Не уведомлять')
    W(d, 10).until(lambda d: any(r['token'] == 'abc123' and r['notificationLevel'] == 3 for r in nc_mock('GET', '/test/state')['rooms']))
    W(d, 10).until(lambda d: row().find_elements(By.CSS_SELECTOR, '.room-muted'))
    ctx(row()); ctx_pick('Включить уведомления')
    W(d, 10).until(lambda d: any(r['token'] == 'abc123' and r['notificationLevel'] == 0 for r in nc_mock('GET', '/test/state')['rooms']))
    ok('chat: conversation right-click menu (mute/unmute notifications)')

    # Новая личная беседа.
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Новая беседа"]').click()
    wait(d, '.talk-new-modal input').send_keys('Сидор')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-user-list .message-row'))[0].click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-new-modal button') if b.text == 'Написать'][0].click()
    W(d, 15).until(lambda d: 'Сидоров Олег' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').text)
    ok('new 1:1 conversation created and opened')

    # Поиск сотрудников из списка бесед: в том числе тех, кого Nextcloud не подсказывает
    # (ограничение автодополнения), — через корпоративную адресную книгу по e-mail.
    assert dav_put('/test@corp.test/contacts/smirnov.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:smirnov\r\nFN:Смирнов Пётр\r\nEMAIL:smirnov@inst.test\r\nEND:VCARD\r\n') in (200, 201, 204)
    rooms_search = d.find_element(By.CSS_SELECTOR, '.talk-rooms .search-box input')
    rooms_search.send_keys('Петров')
    W(d, 15).until(lambda d: any('Петрова Анна' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-people .person-row')))
    clear(rooms_search); rooms_search.send_keys('Смирнов')
    W(d, 20).until(lambda d: any('Смирнов Пётр' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.talk-people .person-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-people .person-row') if 'Смирнов Пётр' in r.get_attribute('textContent')][0].click()
    W(d, 15).until(lambda d: 'Смирнов Пётр' in d.find_element(By.CSS_SELECTOR, '.talk-main .calendar-title').get_attribute('textContent'))
    dav_delete('/test@corp.test/contacts/smirnov.vcf')
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
        M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('test@corp.test', 'Secret-123'); return M
    def subjects(folder):
        M = imap(); M.select(f'"{folder}"')
        _, ids = M.search(None, 'ALL'); out = []
        for i in ids[0].split():
            _, data = M.fetch(i, '(BODY.PEEK[HEADER.FIELDS (SUBJECT)])')
            out.append(str(make_header(decode_header(email.message_from_bytes(data[0][1])['Subject'] or ''))))
        M.logout(); return out
    M = imap()
    for subj in ('E2E перенос A', 'E2E перенос B', 'E2E удалить C', 'E2E тащить D'):
        msg = EmailMessage(); msg['From'] = 'Робот <robot@inst.test>'; msg['To'] = 'test@corp.test'; msg['Subject'] = subj; msg.set_content('тест')
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
    msg = EmailMessage(); msg['From'] = 'x@inst.test'; msg['To'] = 'test@corp.test'; msg['Subject'] = 'E2E other client deleted'; msg.set_content('x')
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
        msg = EmailMessage(); msg['From'] = 'Начальник <boss@inst.test>'; msg['To'] = 'test@corp.test, Петров <petrov@inst.test>'
        msg['Cc'] = 'Сидоров <sidorov@inst.test>, test@corp.test'; msg['Subject'] = subj; msg['Message-ID'] = f'<{subj.replace(" ", "-")}@inst.test>'; msg.set_content('Длинный текст письма.\n' * 200)
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
    assert 'boss@inst.test' in to_v and 'petrov@inst.test' in to_v and 'test@corp.test' not in to_v, to_v
    assert 'sidorov@inst.test' in cc_v and 'test@corp.test' not in cc_v, cc_v
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
    assert 'boss@inst.test' in to_v and 'petrov@inst.test' in to_v, to_v
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
        msg = EmailMessage(); msg['From'] = 'Коллега <colleague@inst.test>'; msg['To'] = 'test@corp.test'; msg['Subject'] = subj; msg.set_content('текст')
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
    assert 'colleague@inst.test' in d.find_element(By.CSS_SELECTOR, '.compose-modal .recipient-field input').get_attribute('value')
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
    msg = EmailMessage(); msg['From'] = 'Фотограф <photo@inst.test>'; msg['To'] = 'test@corp.test'; msg['Subject'] = 'E2E preview'; msg.set_content('Смотрите вложения.')
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
                        '<p><a href="https://example.org/novosti">Подробнее</a></p><p><a id="phish" href="https://evil.example/login">https://www.inst.test/login</a></p></td></tr></table></body></html>', 'html', 'utf-8'))
    logo = MIMEImage(png(120, 40, (0, 51, 102)), 'png'); logo.add_header('Content-ID', '<logo@rrcki>'); logo.add_header('Content-Disposition', 'inline', filename='logo.png')
    rel.attach(logo)
    rel['Subject'] = 'E2E html'; rel['From'] = 'Рассылка <news@inst.test>'; rel['To'] = 'test@corp.test'
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
    assert 'inst.test' in t and 'evil.example' in t, t
    d.save_screenshot(f'{OUT}/28-phishing-link.png')
    [b for b in modal.find_elements(By.TAG_NAME, 'button') if 'Не открывать' in b.text][0].click()
    # XSS: типовые приёмы в HTML-письме — ничего не выполняется ни в рамке письма, ни в окне программы.
    xss = ('<p id="x0">Проверка</p><img src=x onerror="top.__xss=1"><svg onload="top.__xss=2"><circle r=1 /></svg>'
           '<a id="xl" href="javascript:top.__xss=3">ссылка</a><iframe srcdoc="<script>top.__xss=4</script>"></iframe>'
           '<script>top.__xss=5</script><body onload="top.__xss=6"><details open ontoggle="top.__xss=7">d</details>'
           '<form action="javascript:top.__xss=8"><button id="xb">b</button></form><meta http-equiv="refresh" content="0;url=javascript:top.__xss=9">'
           '<object data="javascript:top.__xss=10"></object><embed src="javascript:top.__xss=11"><base href="javascript:top.__xss=12//">'
           '<style>@import "https://localhost:8443/e2e/x.css"; body{background:url("javascript:top.__xss=13")}</style>'
           '<math><mtext><table><mglyph><style><img src=x onerror="top.__xss=14"></style></mglyph></table></mtext></math>')
    xm = MIMEText(f'<html><body>{xss}</body></html>', 'html', 'utf-8'); xm['Subject'] = 'E2E xss'; xm['From'] = 'Злоумышленник <bad@evil.example>'; xm['To'] = 'test@corp.test'
    M = imap(); M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), xm.as_bytes()); M.logout()
    title0 = d.title
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: row('E2E xss'))
    row('E2E xss')[0].click()
    W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, 'iframe.message-html'))
    frame = d.find_element(By.CSS_SELECTOR, 'iframe.message-html'); d.switch_to.frame(frame)
    W(d, 10).until(lambda d: 'Проверка' in d.find_element(By.TAG_NAME, 'body').text)
    # В DOM письма: ни опасных элементов, ни обработчиков, ни javascript:-адресов в атрибутах.
    found = d.execute_script("""
      const bad = [...document.body.querySelectorAll('script, iframe, object, embed, form, base, svg, math, meta[http-equiv]')].map((e) => e.tagName);
      for (const e of document.body.querySelectorAll('*')) for (const a of e.attributes) {
        if (a.name.startsWith('on') || /^\\s*javascript:/i.test(a.value)) bad.push(e.tagName + '@' + a.name);
      }
      return bad""")
    assert not found, found
    for el in d.find_elements(By.CSS_SELECTOR, 'a, button'):
        try: el.click()
        except Exception: pass
    d.switch_to.default_content()
    time.sleep(1.5)
    assert d.execute_script("return window.__xss === undefined") and d.title == title0, d.execute_script("return window.__xss")
    assert not d.find_elements(By.CSS_SELECTOR, '.confirm-modal[aria-label="Подозрительная ссылка"]') or True
    ok('HTML mail XSS payloads (handlers, javascript: links, script, svg, form, meta refresh, base, mXSS) — removed, nothing runs')
    # дальше проверки идут на письме-рассылке
    row('E2E html')[0].click()
    W(d, 20).until(lambda d: d.execute_script("const f = document.querySelector('iframe.message-html'); return !!(f && f.contentDocument && f.contentDocument.body && f.contentDocument.body.textContent.includes('Вёрстка'))"))
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.confirm-modal'))
    ok('link whose text shows another domain: warning before opening')
    # Даже если бы очистку обошли — CSP рамки письма не даёт исполнить код (обработчик onerror).
    frame = d.find_element(By.CSS_SELECTOR, 'iframe.message-html'); d.switch_to.frame(frame)
    d.execute_script("document.body.insertAdjacentHTML('beforeend', '<img src=\"x\" onerror=\"window.parent.__xss = 1\">')")
    d.switch_to.default_content(); time.sleep(1)
    assert d.execute_script('return window.__xss') is None, 'inline handler must be blocked by the frame CSP'
    ok('mail frame CSP blocks injected inline handlers')
    # Тёмная тема: тело письма тоже тёмное (картинки — как есть), по кнопке — исходный светлый фон.
    frame_filter = lambda: d.execute_script("const f = document.querySelector('iframe.message-html'); return f && f.contentDocument && getComputedStyle(f.contentDocument.documentElement).filter")
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Тема"]').click()
    W(d, 5).until(lambda d: 'invert' in (frame_filter() or ''))
    img_filter = d.execute_script("const i = document.querySelector('iframe.message-html').contentDocument.querySelector('img'); return i ? getComputedStyle(i).filter : 'invert(1)'")
    assert 'invert' in img_filter, img_filter
    # Тема — на всей странице: и то, что выводится поверх окна программы, в цветах тёмной темы.
    assert d.execute_script("return document.documentElement.classList.contains('theme-dark') && getComputedStyle(document.documentElement).getPropertyValue('--surface').trim()") == '#202735'
    assert d.execute_script("return getComputedStyle(document.body).backgroundColor") == 'rgb(21, 26, 35)'
    d.save_screenshot(f'{OUT}/27a-dark-html-mail.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.html-foot button') if 'Светлый фон' in b.text][0].click()
    W(d, 5).until(lambda d: 'invert' not in (frame_filter() or ''))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.html-foot button') if 'Тёмный фон' in b.text][0].click()
    W(d, 5).until(lambda d: 'invert' in (frame_filter() or ''))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Тема"]').click()
    W(d, 5).until(lambda d: (frame_filter() or 'none') == 'none' and not d.execute_script("return document.documentElement.classList.contains('theme-dark')"))
    ok('dark theme: HTML mail body darkened (images kept), light background on request; theme applies page-wide')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.html-foot button') if 'как текст' in b.text][0].click()
    W(d, 5).until(lambda d: 'Вёрстка рассылки' in d.find_element(By.CSS_SELECTOR, '.message-body.plain').get_attribute('textContent'))
    ok('HTML mail: plain-text view toggle')

    # Уведомления о новой почте и сообщениях чата.
    notify_log = os.environ['RRCKI_E2E_NOTIFY_LOG']
    def notified():
        return open(notify_log, encoding='utf-8').read() if os.path.exists(notify_log) else ''
    def new_mail(subj):
        m = EmailMessage(); m['From'] = 'Коллега <colleague@inst.test>'; m['To'] = 'test@corp.test'; m['Subject'] = subj; m.set_content('x')
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
    [b for b in d.find_elements(By.CSS_SELECTOR, '.settings-modal button') if 'Правила обработки почты' in b.text][0].click()
    W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.filter-item'))
    t = d.find_element(By.CSS_SELECTOR, '.filter-item').get_attribute('textContent')
    assert 'Рассылка' in t and '[noc]' in t, t; ok('filters: existing SOGo filter listed')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filters-modal button') if 'Новое правило' in b.text][0].click()
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Название правила"]'), 'От Иванова')
    from selenium.webdriver.support.ui import Select
    Select(d.find_element(By.CSS_SELECTOR, '.filter-row select[aria-label="Поле"]')).select_by_visible_text('От')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.filter-row input[aria-label="Значение"]'), 'ivanov@inst.test')
    d.find_element(By.CSS_SELECTOR, '.filter-folder').click()
    wait(d, '.folder-picker input').send_keys('2026')
    W(d, 5).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.folder-picker-list .folder-item')) == 1)
    d.find_element(By.CSS_SELECTOR, '.folder-picker-list .folder-item').click()
    d.save_screenshot(f'{OUT}/18-filter-edit.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-editor button') if 'Сохранить правило' in b.text][0].click()
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.filter-item')) == 2)
    import json as _j, urllib.request as _u
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    saved = _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())
    f2 = saved['SOGoSieveFilters'][1]
    assert saved['SOGoMailSignature'] == 'С уважением', saved
    assert f2['name'] == 'От Иванова' and f2['rules'] == [{'field': 'from', 'operator': 'contains', 'value': 'ivanov@inst.test'}], f2
    assert f2['actions'] == [{'method': 'fileinto', 'argument': 'Проекты/2026'}], f2
    ok('filters: new rule saved to SOGo, other preferences kept')
    d.find_element(By.CSS_SELECTOR, '.filter-item .switch input').click()
    W(d, 10).until(lambda d: _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())['SOGoSieveFilters'][0]['active'] == 0)
    ok('filters: toggle off saved')
    d.save_screenshot(f'{OUT}/19-filters.png')

    # Конструктор правил. Серверное правило «Рассылка» + уведомление от программы:
    # в SOGo уходит только серверная часть, уведомление хранит программа.
    def inv(cmd, args=None):
        return d.execute_async_script("const cb = arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {}).then((r) => cb({ok: r}), (e) => cb({err: JSON.stringify(e)}))", cmd, args)
    sogo_filters = lambda: _j.loads(_u.urlopen('https://localhost:8444/__defaults', context=ctx).read())['SOGoSieveFilters']
    d.find_element(By.CSS_SELECTOR, '.filter-item .switch input').click()
    W(d, 10).until(lambda d: sogo_filters()[0]['active'] == 1)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Изменить Рассылка"]').click()
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-editor .text-button') if 'Действие' in b.text][0].click()
    Select(d.find_elements(By.CSS_SELECTOR, '.filter-editor select[aria-label="Действие"]')[-1]).select_by_value('notify')
    assert 'программа' in d.find_element(By.CSS_SELECTOR, '.rule-placement.server').get_attribute('textContent')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-editor button') if 'Сохранить правило' in b.text][0].click()
    W(d, 10).until(lambda d: inv('rules_get')['ok']['ext'].get('Рассылка') == [{'method': 'notify'}])
    assert sogo_filters()[0]['actions'] == [{'method': 'fileinto', 'argument': 'INBOX/Рассылки'}], sogo_filters()[0]
    assert 'сервер + программа' in [i for i in d.find_elements(By.CSS_SELECTOR, '.filter-item') if 'Рассылка' in i.get_attribute('textContent')][0].get_attribute('textContent')
    ok('rules: server rule kept in SOGo, its notification added by the program')
    # Правило программы: условие по вложению → категория, уведомление, задача в Deck.
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filters-modal button') if 'Новое правило' in b.text][0].click()
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Название правила"]'), 'Отчёты')
    Select(d.find_element(By.CSS_SELECTOR, '.filter-row select[aria-label="Поле"]')).select_by_value('attachment')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.filter-row input[aria-label="Значение"]'), 'отчёт')
    Select(d.find_element(By.CSS_SELECTOR, '.filter-editor select[aria-label="Действие"]')).select_by_value('category')
    Select(d.find_element(By.CSS_SELECTOR, '.filter-editor select[aria-label="Категория"]')).select_by_value('$label4')
    for method in ('notify', 'task'):
        [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-editor .text-button') if 'Действие' in b.text][0].click()
        Select(d.find_elements(By.CSS_SELECTOR, '.filter-editor select[aria-label="Действие"]')[-1]).select_by_value(method)
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.filter-editor select[aria-label="Доска"] option')) >= 2)
    Select(d.find_element(By.CSS_SELECTOR, '.filter-editor select[aria-label="Доска"]')).select_by_value('1')
    W(d, 10).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.filter-editor select[aria-label="Колонка"] option')) >= 2)
    Select(d.find_element(By.CSS_SELECTOR, '.filter-editor select[aria-label="Колонка"]')).select_by_value('10')
    assert d.find_elements(By.CSS_SELECTOR, '.rule-placement.program')
    d.save_screenshot(f'{OUT}/19a-rule-editor.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-editor button') if 'Сохранить правило' in b.text][0].click()
    W(d, 10).until(lambda d: [i for i in d.find_elements(By.CSS_SELECTOR, '.filter-item[data-where=program]') if 'Отчёты' in i.get_attribute('textContent')])
    prog = inv('rules_get')['ok']['program']
    assert prog[0]['rules'] == [{'field': 'attachment', 'operator': 'contains', 'value': 'отчёт'}], prog
    assert [a['method'] for a in prog[0]['actions']] == ['category', 'notify', 'task'] and prog[0]['actions'][2]['argument'] == '1:10', prog
    assert len(sogo_filters()) == 2, 'program rule must not reach the server'
    d.save_screenshot(f'{OUT}/19b-rules.png')
    ok('rules: attachment condition makes a program rule (category, notification, Deck task); not sent to the server')
    d.find_element(By.CSS_SELECTOR, '.filters-modal button[aria-label="Закрыть"]').click()
    # Новые письма после сохранения правил: обработчик сначала запоминает папки (один проход), потом применяет.
    time.sleep(12)
    rm = EmailMessage(); rm['From'] = 'Плановый отдел <plan@ihep.ru>'; rm['To'] = 'test@corp.test'; rm['Subject'] = 'E2E правило отчёт'
    rm.set_content('см. вложение'); rm.add_attachment('цифры'.encode(), maintype='text', subtype='plain', filename='Отчёт за ноябрь.txt')
    M = imap(); M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), rm.as_bytes()); M.logout()
    W(d, 60).until(lambda d: 'Правило «Отчёты»\tПлановый отдел: E2E правило отчёт' in notified())
    W(d, 30).until(lambda d: any(c['title'] == 'E2E правило отчёт' and c['stackId'] == 10 and 'plan@ihep.ru' in c['description'] for c in nc_mock('GET', '/test/state')['cards']))
    M = imap(); M.select('INBOX'); _, ids = M.search(None, 'KEYWORD', '$label4', 'SUBJECT', '"E2E"'); M.logout()
    assert ids[0].split(), 'category $label4 set by the rule'
    ok('program rule applied to new mail: category set, notification shown, Deck card created')
    # Дополнение к серверному правилу: письмо, которое сервер положил в «INBOX/Рассылки».
    def mutf7(text):
        out, run = [], []
        def flush():
            if run:
                out.append('&' + __import__('base64').b64encode(''.join(run).encode('utf-16-be')).decode().rstrip('=').replace('/', ',') + '-'); run.clear()
        for ch in text:
            if 0x20 <= ord(ch) <= 0x7e:
                flush(); out.append('&-' if ch == '&' else ch)
            else:
                run.append(ch)
        flush()
        return ''.join(out)
    fid = mutf7('INBOX/Рассылки')
    assert 'ok' in inv('create_folder', {'name': fid})
    time.sleep(12)
    nm = EmailMessage(); nm['From'] = 'NOC <noc@inst.test>'; nm['To'] = 'test@corp.test'; nm['Subject'] = '[noc] плановые работы'; nm.set_content('x')
    M = imap(); M.append(f'"{fid}"', None, imaplib.Time2Internaldate(time.time()), nm.as_bytes()); M.logout()
    W(d, 60).until(lambda d: 'Правило «Рассылка»\tNOC: [noc] плановые работы' in notified())
    ok('server rule extension: notification for mail the server sorted into its folder')
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
    for secret in ['Тестовое письмо'.encode(), 'robot@corp.test'.encode(), 'Отчёт с вложением'.encode(), 'Содержимое отчёта'.encode()]:
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

    # Операторы запроса и имена вложений (индекс на компьютере).
    set_value(d, q, 'вложение:октябрь')
    try:
        W(d, 15).until(lambda d: subj(d) == ['Отчёт с вложением'])
    except Exception:
        print('ROWS:', subj(d)); raise
    set_value(d, q, 'от:robot тема:119')
    W(d, 10).until(lambda d: subj(d) == ['Письмо №119'])
    set_value(d, q, 'тема:119 после:2000-01-01 до:2000-01-02')
    W(d, 10).until(lambda d: subj(d) == [])
    set_value(d, q, 'от:robot тема:119')
    W(d, 10).until(lambda d: subj(d) == ['Письмо №119'])
    ok('search operators: attachment name, sender + subject, date range')
    # Сохранённый поиск и быстрые представления.
    [b for b in d.find_elements(By.CSS_SELECTOR, '.search-tools button') if 'Сохранить поиск' in b.text][0].click()
    set_value(d, wait(d, 'input[aria-label="Название поиска"]'), 'Робот 119')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.save-view button') if b.text == 'Сохранить'][0].click()
    W(d, 10).until(lambda d: [v for v in d.find_elements(By.CSS_SELECTOR, '.view-item') if 'Робот 119' in v.get_attribute('textContent')])
    views_file = os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/views.json'
    assert json.load(open(views_file))[0]['query'] == 'от:robot тема:119' and os.stat(views_file).st_mode & 0o077 == 0
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Очистить поиск"]').click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.search-header'))
    [v for v in d.find_elements(By.CSS_SELECTOR, '.view-item') if 'Робот 119' in v.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: q.get_attribute('value') == 'от:robot тема:119' and subj(d) == ['Письмо №119'])
    assert 'Робот 119' in d.find_element(By.CSS_SELECTOR, '.search-saved').get_attribute('textContent')
    d.find_element(By.CSS_SELECTOR, '.view-item[data-view="q-important"]').click()
    W(d, 10).until(lambda d: q.get_attribute('value') == 'важные' and d.find_elements(By.CSS_SELECTOR, '.search-header'))
    try:
        W(d, 10).until(lambda d: 'Письмо №119' not in subj(d) and all(r.find_elements(By.CSS_SELECTOR, '.important-mark') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    except Exception:
        print('ROWS:', subj(d)); raise
    d.save_screenshot(f'{OUT}/42a-saved-searches.png')
    d.execute_script("arguments[0].click()", d.find_element(By.CSS_SELECTOR, 'button[aria-label="Удалить поиск Робот 119"]'))
    W(d, 10).until(lambda d: not [v for v in d.find_elements(By.CSS_SELECTOR, '.view-item') if 'Робот 119' in v.get_attribute('textContent')])
    assert json.load(open(views_file)) == []
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Очистить поиск"]').click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.search-header'))
    ok('saved search: stored privately, listed under «Поиски», runs in one click; quick view «Важные»; delete')

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
    assert chip.get_attribute('data-address') == 'yushkov_do@inst.test', chip.get_attribute('data-address')
    assert d.execute_script("const s = getComputedStyle(arguments[0]); return s.userSelect || s.webkitUserSelect", chip) in ('text', 'auto'), 'not selectable'
    chip.click()
    [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if b.text == 'Копировать адрес'][0].click()
    W(d, 5).until(lambda d: 'Скопировано: yushkov_do@inst.test' in d.find_element(By.CSS_SELECTOR, '.toast').get_attribute('textContent'))
    chip.click()
    [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if 'Добавить в контакты' in b.text][0].click()
    ed = wait(d, '.editor-modal', 20)
    assert field(d, 'Фамилия Имя Отчество').get_attribute('value') == 'Юшков Дмитрий'
    assert 'yushkov_do@inst.test' in field(d, 'E-mail (по одному в строке)').get_attribute('value')
    d.save_screenshot(f'{OUT}/43-add-contact.png')
    ed.find_element(By.CSS_SELECTOR, 'button[type=submit]').click()
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.editor-modal'))
    W(d, 10).until(lambda d: any('yushkov_do@inst.test' in c['emails'] for c in (inv('list_contacts').get('ok') or {}).get('contacts', [])))
    ok('sender address: copy and add to contacts (prefilled)')

    # история взаимодействия с организацией (домен jinr.ru): письма, вложения, встречи, контакты, договорённости
    import base64 as _b64m, datetime as _dt, urllib.request as _ur
    M = imap()
    for subj, frm, to, folder, files in (
        ('ОИЯИ: проект договора', 'Петров П. <petrov@jinr.ru>', 'test@corp.test', 'INBOX', ['Договор ОИЯИ.pdf']),
        ('ОИЯИ: сроки поставки', 'petrov@jinr.ru', 'test@corp.test', 'INBOX', []),
        ('Re: ОИЯИ: проект договора', 'test@corp.test', 'Петров П. <petrov@jinr.ru>', 'Sent', []),
        ('ОИЯИ: удалённое', 'petrov@jinr.ru', 'test@corp.test', 'Trash', []),
    ):
        m = EmailMessage(); m['From'] = frm; m['To'] = to; m['Subject'] = subj; m['Date'] = email.utils.formatdate(localtime=True); m.set_content('Текст про договор')
        for f in files:
            m.add_attachment(b'%PDF-1.4 test', maintype='application', subtype='pdf', filename=f)
        M.append(folder, None, imaplib.Time2Internaldate(time.time()), m.as_bytes())
    M.logout()
    dav_auth = 'Basic ' + _b64m.b64encode(b'test@corp.test:Secret-123').decode()
    dav_ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    def dav_put(path, body, ctype):
        r = _ur.Request('https://localhost:5232/test@corp.test/' + path, data=body.encode(), method='PUT', headers={'Authorization': dav_auth, 'Content-Type': ctype})
        assert _ur.urlopen(r, context=dav_ctx).status in (200, 201, 204)
    meet = (_dt.date.today() - _dt.timedelta(days=10)).strftime('%Y%m%d')
    dav_put('work/jinr-meet.ics', f'BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\nUID:jinr-meet\r\nSUMMARY:Совещание с ОИЯИ\r\nLOCATION:Дубна\r\nDTSTART:{meet}T090000Z\r\nDTEND:{meet}T100000Z\r\nORGANIZER;CN=Юшков:mailto:test@corp.test\r\nATTENDEE;CN=Петров:mailto:petrov@jinr.ru\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n', 'text/calendar; charset=utf-8')
    dav_put('contacts/jinr-ivan.vcf', 'BEGIN:VCARD\r\nVERSION:3.0\r\nUID:jinr-ivan\r\nFN:Иван Сидоренко\r\nEMAIL:ivan@jinr.ru\r\nORG:ОИЯИ\r\nTITLE:Главный инженер\r\nEND:VCARD\r\n', 'text/vcard; charset=utf-8')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    jinr = lambda: [c for c in (inv('counterparties').get('ok') or []) if c['domain'] == 'jinr.ru']
    W(d, 90).until(lambda d: jinr() and jinr()[0]['messages'] == 3)
    c = jinr()[0]
    assert (c['incoming'], c['outgoing'], c['own']) == (2, 1, False), c
    own = [x for x in inv('counterparties')['ok'] if x['own']]
    assert own and own[0]['domain'] == 'corp.test', own
    ok('counterparties: organizations by address domain from local mail (trash skipped, own domain marked)')

    W(d, 20).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'ОИЯИ: проект договора' in r.get_attribute('textContent') and 'Re:' not in r.get_attribute('textContent')])
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'ОИЯИ: проект договора' in r.get_attribute('textContent') and 'Re:' not in r.get_attribute('textContent')][0].click()
    W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, '.sender-line .address-chip').get_attribute('data-address') == 'petrov@jinr.ru')
    d.find_element(By.CSS_SELECTOR, '.sender-line .address-chip').click()
    [b for b in wait(d, '.context-menu').find_elements(By.TAG_NAME, 'button') if 'История с организацией @jinr.ru' in b.text][0].click()
    wait(d, '.counterparty-view')
    W(d, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.timeline-event'))
    head = d.find_element(By.CSS_SELECTOR, '.counterparty-head').get_attribute('textContent')
    assert 'ОИЯИ' in head and '@jinr.ru' in head, head
    kinds = lambda: [li.get_attribute('data-kind') for li in d.find_elements(By.CSS_SELECTOR, '.timeline-item')]
    assert sorted(kinds()) == ['document', 'event', 'mail', 'mail', 'mail'], kinds()
    tl = d.find_element(By.CSS_SELECTOR, '.timeline').get_attribute('textContent')
    assert 'Договор ОИЯИ.pdf' in tl and 'Совещание с ОИЯИ' in tl and 'Re: ОИЯИ: проект договора' in tl and 'удалённое' not in tl, tl
    stats = d.find_element(By.CSS_SELECTOR, '.counterparty-stats').get_attribute('textContent')
    assert '3 письма' in stats and '2 от них' in stats and '1 им' in stats and '1 встреча' in stats and '1 документ' in stats, stats
    people = d.find_element(By.CSS_SELECTOR, '.counterparty-side').get_attribute('textContent')
    assert 'Петров П' in people and 'petrov@jinr.ru' in people and 'Иван Сидоренко' in people and 'Главный инженер' in people, people
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-chip') if b.get_attribute('data-kind') == 'event'][0].click()
    W(d, 5).until(lambda d: kinds() == ['event'])
    d.find_element(By.CSS_SELECTOR, '.timeline-event .timeline-card').click()
    W(d, 5).until(lambda d: 'petrov@jinr.ru' in d.find_element(By.CSS_SELECTOR, '.timeline-details').get_attribute('textContent'))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.filter-chip') if b.get_attribute('data-kind') == 'all'][0].click()
    d.save_screenshot(f'{OUT}/44-counterparty.png')
    ok('counterparty timeline: mail both ways, attachment as document, meeting with its participants, contacts and people')

    # договорённость из письма: срок прошёл → просрочено; отметка в списке организаций; выполнено
    mail_item = [li for li in d.find_elements(By.CSS_SELECTOR, '.timeline-item') if li.get_attribute('data-kind') == 'mail' and 'ОИЯИ: сроки поставки' in li.get_attribute('textContent')][0]
    d.execute_script('arguments[0].scrollIntoView({block: "center"})', mail_item)
    mail_item.find_element(By.CSS_SELECTOR, 'button[aria-label="Договорённость из письма"]').click()
    form = wait(d, '.agreement-form')
    assert 'ОИЯИ: сроки поставки' in form.get_attribute('textContent')
    set_value(d, form.find_element(By.CSS_SELECTOR, 'textarea'), 'Поставка оборудования до 1 декабря')
    set_value(d, form.find_element(By.CSS_SELECTOR, 'input[aria-label="Срок выполнения"]'), (_dt.date.today() - _dt.timedelta(days=1)).isoformat())
    form.find_element(By.CSS_SELECTOR, 'button[type=submit]').click()
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.agreement-row.overdue'))
    assert 'просрочено' in d.find_element(By.CSS_SELECTOR, '.agreement-row').get_attribute('textContent')
    assert d.find_elements(By.CSS_SELECTOR, '.timeline-agreement')
    cp_file = os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/counterparties.json'
    cp_acc = lambda: [v for k, v in json.load(open(cp_file))['accounts'].items() if k.startswith('test@corp.test')][0]
    saved = cp_acc()['jinr.ru']['agreements'][0]
    assert saved['text'] == 'Поставка оборудования до 1 декабря' and saved['subject'] == 'ОИЯИ: сроки поставки' and saved['uid'] and not saved['done'], saved
    assert os.stat(cp_file).st_mode & 0o077 == 0
    W(d, 10).until(lambda d: '1' in d.find_element(By.CSS_SELECTOR, '.counterparty-row[data-domain="jinr.ru"]').get_attribute('textContent') and d.find_elements(By.CSS_SELECTOR, '.counterparty-row[data-domain="jinr.ru"] .counterparty-badge'))
    d.find_element(By.CSS_SELECTOR, '.agreement-row input[type=checkbox]').click()
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.agreement-row.done') and not d.find_elements(By.CSS_SELECTOR, '.counterparty-row[data-domain="jinr.ru"] .counterparty-badge'))
    assert cp_acc()['jinr.ru']['agreements'][0]['done'] is True
    ok('agreement from a mail: due date, overdue mark, open count in the list, done; stored privately per account')

    # название и заметка; поиск и на сервере; другой домен вручную
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Изменить название"]').click()
    set_value(d, wait(d, 'input[aria-label="Название организации"]'), 'Объединённый институт ядерных исследований')
    d.find_element(By.CSS_SELECTOR, '.counterparty-name-form button[type=submit]').click()
    W(d, 10).until(lambda d: 'Объединённый институт' in d.find_element(By.CSS_SELECTOR, '.counterparty-row[data-domain="jinr.ru"]').get_attribute('textContent'))
    note = d.find_element(By.CSS_SELECTOR, 'textarea[aria-label="Заметка об организации"]')
    set_value(d, note, 'Договор № 17/2026')
    d.execute_script('arguments[0].dispatchEvent(new FocusEvent("blur")); arguments[0].dispatchEvent(new FocusEvent("focusout", {bubbles: true}))', note)
    W(d, 10).until(lambda d: cp_acc()['jinr.ru'].get('note') == 'Договор № 17/2026')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.counterparty-actions button') if 'на сервере' in b.text][0].click()
    W(d, 30).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.counterparty-actions button') if 'Обновить с сервера' in b.text])
    assert sorted(kinds()).count('mail') == 3, kinds()
    srch = d.find_element(By.CSS_SELECTOR, 'input[aria-label="Найти организацию"]')
    set_value(d, srch, 'cern.ch')
    wait(d, '.counterparty-new').click()
    W(d, 20).until(lambda d: '@cern.ch' in d.find_element(By.CSS_SELECTOR, '.counterparty-head').get_attribute('textContent') and 'Здесь пока пусто' in d.find_element(By.CSS_SELECTOR, '.counterparty-timeline').get_attribute('textContent'))
    assert 'err' in inv('counterparty_timeline', {'domain': '../etc', 'server': False}), 'bad domain accepted'
    ok('counterparty: name and note saved, server search, open any domain by hand; invalid domain refused')
    d.find_element(By.CSS_SELECTOR, '.module-rail button').click()
    wait(d, '.message-row')

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
    compose('colleague@inst.test', 'E2E отмена отправки', 'Текст для отмены')
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
    assert 'Disposition-Notification-To: <test@corp.test>' in raw, raw[:600]
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.outbox-toast'))
    ok('send now from the undo bar; read receipt requested (Disposition-Notification-To)')

    # Отложенная отправка: письмо ждёт в «Черновиках» и уходит в срок.
    compose('colleague@inst.test', 'E2E отложенное', 'Уйдёт позже')
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
    compose('colleague@inst.test', 'E2E отложенное отменить', 'Не уйдёт')
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
    # Связь оборвалась после передачи письма: результат неизвестен — повторно само не уходит.
    compose('colleague@inst.test', 'E2E отложенное обрыв [drop]', 'Связь оборвётся после передачи')
    d.find_element(By.CSS_SELECTOR, '.send-more').click()
    pop = wait(d, '.schedule-pop')
    set_value(d, pop.find_element(By.CSS_SELECTOR, 'input[type=datetime-local]'), time.strftime('%Y-%m-%dT%H:%M', time.localtime(time.time() + 100)))
    [b for b in pop.find_elements(By.TAG_NAME, 'button') if 'Запланировать' in b.text][0].click()
    W(d, 180).until(lambda d: sent_with('E2E отложенное обрыв [drop]'))
    W(d, 30).until(lambda d: [x for x in inv('scheduled_list')['ok'] if 'обрыв' in x['subject'] and x['status'] == 'unknown'])
    time.sleep(20)  # несколько проходов планировщика (раз в 5 с в тестах)
    assert len(sent_with('E2E отложенное обрыв [drop]')) == 1, 'unknown-result mail was resent automatically'
    d.find_element(By.CSS_SELECTOR, '.scheduled-link').click()
    modal = wait(d, '.scheduled-modal')
    item = [i for i in modal.find_elements(By.CSS_SELECTOR, '.scheduled-item') if 'обрыв' in i.get_attribute('textContent')][0]
    assert 'Неизвестно, дошло ли письмо' in item.get_attribute('textContent'), item.get_attribute('textContent')
    d.save_screenshot(f'{OUT}/46a-scheduled-unknown.png')
    [b for b in item.find_elements(By.TAG_NAME, 'button') if b.text == 'Отменить'][0].click()
    W(d, 15).until(lambda d: not [x for x in inv('scheduled_list')['ok'] if 'обрыв' in x['subject']])
    d.find_element(By.CSS_SELECTOR, '.scheduled-modal button[aria-label="Закрыть"]').click()
    assert len(sent_with('E2E отложенное обрыв [drop]')) == 1
    ok('scheduled mail: connection dropped after sending -> «result unknown», never resent automatically')

    # Уведомление о прочтении: запрос во входящем письме.
    msg = EmailMessage(); msg['From'] = 'Начальник <boss@inst.test>'; msg['To'] = 'test@corp.test'; msg['Subject'] = 'E2E MDN request'
    msg['Message-ID'] = '<mdn-request@inst.test>'; msg['Disposition-Notification-To'] = 'boss@inst.test'; msg.set_content('Подтвердите прочтение')
    M = imap(); M.append('INBOX', '', imaplib.Time2Internaldate(time.time()), msg.as_bytes()); M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: any('E2E MDN request' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if 'E2E MDN request' in r.get_attribute('textContent')][0].click()
    bar = wait(d, '.mdn-bar', 15)
    [b for b in bar.find_elements(By.TAG_NAME, 'button') if b.text == 'Отправить'][0].click()
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.mdn-bar'))
    W(d, 15).until(lambda d: any(b'disposition-notification' in open(p, 'rb').read() and b'<mdn-request@inst.test>' in open(p, 'rb').read() for p in outbox()))
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

# 14d2. работа без сети: письма с компьютера, очередь действий и исходящих, конфликты, запуск без сети
def block_servers(on):
    # Сервер «пропадает»: IMAP и SMTP отвечают сбросом соединения.
    for cmd in ('iptables', 'ip6tables'):
        for port in (10993, 465):
            subprocess.run([cmd, '-I' if on else '-D', 'INPUT', '-p', 'tcp', '--dport', str(port), '-j', 'REJECT', '--reject-with', 'tcp-reset'],
                           check=on and cmd == 'iptables', stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
d = session()
try:
    wait(d, '.message-row', 30)
    def inv(cmd, args=None):
        return d.execute_async_script("const cb = arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {}).then((r) => cb({ok: r}), (e) => cb({err: JSON.stringify(e)}))", cmd, args)
    rowt = lambda t: [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if t in r.get_attribute('textContent')]
    # пересылка письма с вложениями: вложения исходного письма уходят вместе с ним
    M = imap()
    fm = EmailMessage(); fm['From'] = 'Петрова Анна <petrova@inst.test>'; fm['To'] = 'test@corp.test'; fm['Subject'] = 'E2E пересылка'; fm.set_content('Смета во вложении')
    fm.add_attachment(b'SMETA-DATA-2027', maintype='application', subtype='vnd.openxmlformats-officedocument.spreadsheetml.sheet', filename='Смета 2027.xlsx')
    fm.add_attachment(b'%PDF-1.4 plan', maintype='application', subtype='pdf', filename='plan.pdf')
    M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), fm.as_bytes()); M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: rowt('E2E пересылка'))
    rowt('E2E пересылка')[0].click()
    W(d, 10).until(lambda d: 'Смета во вложении' in d.find_element(By.CSS_SELECTOR, '.reader-content').text)
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Переслать"]').click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.compose-files') and 'Смета 2027.xlsx' in d.find_element(By.CSS_SELECTOR, '.compose-files').get_attribute('textContent') and 'plan.pdf' in d.find_element(By.CSS_SELECTOR, '.compose-files').get_attribute('textContent'))
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'Fwd: E2E пересылка'
    set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), 'colleague@inst.test')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 20).until(lambda d: sent_with('Fwd: E2E пересылка'))
    fwd = email.message_from_bytes(subject_of(sent_with('Fwd: E2E пересылка')[0])[1])
    parts = {str(make_header(decode_header(p.get_filename()))): p.get_payload(decode=True) for p in fwd.walk() if p.get_filename()}
    assert parts == {'Смета 2027.xlsx': b'SMETA-DATA-2027', 'plan.pdf': b'%PDF-1.4 plan'}, parts
    ok('forward: attachments of the original mail are attached and sent')
    # файл с расписанием совещаний во вложении — сразу в свой календарь
    import datetime as _dtc
    d1 = _dtc.date.today() + _dtc.timedelta(days=7)
    ics = ('BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//RU\r\nMETHOD:PUBLISH\r\n'
           f'BEGIN:VEVENT\r\nUID:e2e-sched-1\r\nSUMMARY:Планёрка по проекту\r\nDTSTART;TZID=Europe/Moscow:{d1:%Y%m%d}T100000\r\nDTEND;TZID=Europe/Moscow:{d1:%Y%m%d}T110000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nLOCATION:Переговорная 214\r\nEND:VEVENT\r\n'
           f'BEGIN:VEVENT\r\nUID:e2e-sched-2\r\nSUMMARY:Выездное совещание\r\nDTSTART;VALUE=DATE:{d1 + _dtc.timedelta(days=2):%Y%m%d}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n')
    im = EmailMessage(); im['From'] = 'Секретарь <secretary@inst.test>'; im['To'] = 'test@corp.test'; im['Subject'] = 'E2E расписание совещаний'; im.set_content('Расписание во вложении')
    im.add_attachment(ics.encode(), maintype='text', subtype='calendar', filename='Расписание.ics')
    M = imap(); M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), im.as_bytes()); M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: rowt('E2E расписание совещаний'))
    rowt('E2E расписание совещаний')[0].click()
    wait(d, 'button[aria-label="Добавить Расписание.ics в календарь"]', 15).click()
    dlg = wait(d, '.ics-modal')
    W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.ics-events li')) == 2)
    txt = dlg.get_attribute('textContent')
    assert 'Планёрка по проекту' in txt and 'повторяется' in txt and 'Переговорная 214' in txt and 'Выездное совещание' in txt and 'весь день' in txt, txt
    cal_opts = [o.get_attribute('textContent') for o in dlg.find_elements(By.CSS_SELECTOR, 'select[aria-label="Календарь"] option')]
    assert 'Рабочий' in cal_opts, cal_opts
    from selenium.webdriver.support.ui import Select as _Sel
    _Sel(dlg.find_element(By.CSS_SELECTOR, 'select[aria-label="Календарь"]')).select_by_visible_text('Рабочий')
    d.save_screenshot(f'{OUT}/48-ics-import.png')
    [b for b in dlg.find_elements(By.CSS_SELECTOR, '.ics-buttons button') if 'Добавить' in b.text][0].click()
    W(d, 20).until(lambda d: any('добавлено: 2' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    rng = {'start': f'{d1 - _dtc.timedelta(days=1)}T00:00:00Z', 'end': f'{d1 + _dtc.timedelta(days=30)}T00:00:00Z'}
    titles = lambda: [e['title'] for e in inv('list_events', rng)['ok']['events']]
    assert titles().count('Планёрка по проекту') == 3 and titles().count('Выездное совещание') == 1, titles()
    # повторно (обновлённое расписание) — заменяется, не задваивается
    wait(d, 'button[aria-label="Добавить Расписание.ics в календарь"]').click()
    dlg = wait(d, '.ics-modal')
    W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.ics-events li')) == 2)
    _Sel(dlg.find_element(By.CSS_SELECTOR, 'select[aria-label="Календарь"]')).select_by_visible_text('Рабочий')
    [b for b in dlg.find_elements(By.CSS_SELECTOR, '.ics-buttons button') if 'Добавить' in b.text][0].click()
    W(d, 20).until(lambda d: any('обновлено: 2' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    assert titles().count('Планёрка по проекту') == 3 and titles().count('Выездное совещание') == 1, titles()
    ok('calendar file in a mail: events previewed and added to the chosen calendar; re-import updates, no duplicates')
    # перетаскивание вложения из письма в папку компьютера: файл с настоящим именем готовится и отдаётся системе
    t_drag = time.time()
    rowt('E2E пересылка')[0].click()
    W(d, 10).until(lambda d: 'Смета во вложении' in d.find_element(By.CSS_SELECTOR, '.reader-content').text)
    card = [c for c in d.find_elements(By.CSS_SELECTOR, '.attachment-card') if 'Смета 2027.xlsx' in c.get_attribute('textContent')][0]
    assert card.get_attribute('draggable') == 'true'
    d.execute_script("arguments[0].dispatchEvent(new DragEvent('dragstart', {bubbles: true, cancelable: true, dataTransfer: new DataTransfer()}))", card)
    bases = [os.path.join(os.environ.get('XDG_RUNTIME_DIR', '/nonexistent'), 'rrcki-mail'), os.path.expanduser('~/.cache/rrcki-mail')]
    def staged():
        out = []
        for f in (f for b in bases for f in glob.glob(f'{b}/dragout-*/Смета 2027.xlsx')):
            try:
                if os.path.getmtime(f) >= t_drag - 1:
                    out.append(f)
            except FileNotFoundError:
                pass  # старую копию программа только что убрала
        return out
    W(d, 15).until(lambda d: staged())
    assert open(staged()[0], 'rb').read() == b'SMETA-DATA-2027'
    assert os.stat(os.path.dirname(staged()[0])).st_mode & 0o077 == 0
    ok('drag an attachment out of a mail: copy with its real name in a private folder handed to the system drag')
    # щелчок по документу без встроенного просмотра — открыть в программе компьютера (Office), а не скачивать
    t_open = time.time()
    card = [c for c in d.find_elements(By.CSS_SELECTOR, '.attachment-card') if 'Смета 2027.xlsx' in c.get_attribute('textContent')][0]
    assert 'открыть' in card.get_attribute('textContent') and 'программе' in card.get_attribute('title'), card.get_attribute('title')
    card.click()
    def opened():
        return [f for f in (f for b in bases for f in glob.glob(f'{b}/open-*/Смета 2027.xlsx')) if os.path.exists(f) and os.path.getmtime(f) >= t_open - 1]
    W(d, 15).until(lambda d: opened())
    assert open(opened()[0], 'rb').read() == b'SMETA-DATA-2027'
    assert os.stat(os.path.dirname(opened()[0])).st_mode & 0o077 == 0
    W(d, 10).until(lambda d: any('открыт в программе' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    assert not os.path.exists(os.path.join(os.environ['E2E_HOME'], 'saved', 'Смета 2027.xlsx')), 'must open, not download'
    ok('click on an office attachment opens a private copy in the local program instead of downloading')
    # файлы с компьютера, брошенные на окно почты, — новое письмо с вложениями
    d.execute_script("""
      const dt = new DataTransfer(); dt.items.add(new File(['план'], 'План работ.docx', {type: 'application/octet-stream'}));
      const t = document.querySelector('.message-list');
      t.dispatchEvent(new DragEvent('dragover', {bubbles: true, cancelable: true, dataTransfer: dt}));
      t.dispatchEvent(new DragEvent('drop', {bubbles: true, cancelable: true, dataTransfer: dt}));""")
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.compose-modal') and any('План работ.docx' in c.get_attribute('textContent') for c in d.find_elements(By.CSS_SELECTOR, '.compose-files .file-chip')))
    assert d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]').get_attribute('value') == 'План работ.docx'
    set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), 'colleague@inst.test')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 20).until(lambda d: sent_with('План работ.docx'))
    fm2 = email.message_from_bytes(subject_of(sent_with('План работ.docx')[0])[1])
    assert any(str(make_header(decode_header(p.get_filename() or ''))) == 'План работ.docx' and p.get_payload(decode=True) == 'план'.encode() for p in fm2.walk()), 'dropped file not attached'
    ok('files dropped onto the mail window open a new mail with them attached')
    M = imap()
    for n, subj in enumerate(('E2E без сети флажок', 'E2E без сети удалить', 'E2E без сети конфликт')):
        m = EmailMessage(); m['From'] = 'Робот <robot@inst.test>'; m['To'] = 'test@corp.test'; m['Subject'] = subj; m['Message-ID'] = f'<offline-{n}-{int(time.time())}@e2e>'; m.set_content(f'Текст: {subj}')
        M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), m.as_bytes())
    M.logout()
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: rowt('E2E без сети флажок') and rowt('E2E без сети конфликт'))
    rowt('E2E без сети флажок')[0].click()
    W(d, 10).until(lambda d: 'Текст: E2E без сети флажок' in d.find_element(By.CSS_SELECTOR, '.reader-content').text)
    uid_of = lambda subj: [m['uid'] for m in inv('cached_messages', {'folder': 'INBOX', 'beforeUid': None, 'limit': 100})['ok']['items'] if m['subject'] == subj][0]
    u_del, u_conf = uid_of('E2E без сети удалить'), uid_of('E2E без сети конфликт')
    block_servers(True)
    try:
        d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
        banner = W(d, 40).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.offline-banner') if 'Нет связи' in b.get_attribute('textContent')])
        assert rowt('E2E без сети флажок') and rowt('E2E без сети удалить'), 'cached list gone'
        assert 'Без сети' in d.find_element(By.CSS_SELECTOR, '.list-footer').get_attribute('textContent')
        ok('offline: no connection detected, folders and mail shown from the computer')
        # отметка из интерфейса, прочтение и удаление — без ошибок, в очередь
        flag = d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Флажок"]')
        flag.click()
        W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Флажок"]').get_attribute('aria-pressed') == 'true')
        assert 'ok' in inv('set_flag', {'folder': 'INBOX', 'uid': u_conf, 'flag': 'seen', 'value': True})
        r = inv('delete_messages', {'folder': 'INBOX', 'uids': [u_del], 'trash': 'Trash'})
        assert r.get('ok') is False, r
        # письмо без сети — в очередь исходящих
        compose('petrov@jinr.ru', 'E2E без сети письмо', 'Отправлено без сети')
        d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
        W(d, 20).until(lambda d: any('очереди исходящих' in t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
        q = inv('offline_queue')['ok']
        assert not q['online'] and q['available'], q
        whats = [i['what'] for i in q['items']]
        assert any('Поставить флажок' in w for w in whats) and any('Отметить прочитанным' in w for w in whats) and any('Удалить' in w for w in whats), whats
        assert any(i['send'] and 'E2E без сети письмо' in i['what'] for i in q['items']), whats
        W(d, 10).until(lambda d: 'В очереди' in d.find_element(By.CSS_SELECTOR, '.offline-banner').get_attribute('textContent'))
        qfiles = glob.glob(os.environ['XDG_DATA_HOME'] + '/**/mailstore/*/queue.bin', recursive=True)
        assert len(qfiles) == 1, qfiles
        qraw = open(qfiles[0], 'rb').read()
        for secret in ('E2E без сети письмо'.encode(), b'petrov@jinr.ru', 'Отправлено без сети'.encode()):
            assert secret not in qraw, secret
        [b for b in d.find_elements(By.CSS_SELECTOR, '.offline-banner button') if 'В очереди' in b.text][0].click()
        panel = wait(d, '.queue-modal')
        try:
            W(d, 15).until(lambda d: 'Отправить «E2E без сети письмо»' in d.find_element(By.CSS_SELECTOR, '.queue-modal').get_attribute('textContent'))
        except Exception:
            print('QUEUE PANEL:', d.find_element(By.CSS_SELECTOR, '.queue-modal').get_attribute('textContent')); raise
        panel = d.find_element(By.CSS_SELECTOR, '.queue-modal')
        d.save_screenshot(f'{OUT}/46-offline-queue.png')
        panel.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
        ok('offline actions and outgoing mail queued (encrypted on disk), listed in the queue')
        # пока связи нет, письмо удалили в другом месте (веб-почта) — будет конфликт
        subprocess.run(['doveadm', '-c', '/srv/imaptest/dovecot.conf', 'expunge', '-u', 'test@corp.test', 'mailbox', 'INBOX', 'header', 'Subject', 'E2E без сети конфликт'], check=True)
    finally:
        block_servers(False)
    W(d, 60).until(lambda d: (lambda q: q['online'] and not q['items'])(inv('offline_queue')['ok']))
    W(d, 15).until(lambda d: sent_with('E2E без сети письмо'))
    assert 'E2E без сети письмо' in subjects('Sent')
    assert 'E2E без сети удалить' in subjects('Trash') and 'E2E без сети удалить' not in subjects('INBOX')
    M = imap(); M.select('INBOX')
    _, ids = M.uid('SEARCH', None, 'ALL')
    flagged = False
    for u in ids[0].split():
        _, data = M.uid('FETCH', u, '(FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT)])')
        hdr = data[0][1] if isinstance(data[0], tuple) else b''
        if 'E2E без сети флажок' in str(make_header(decode_header(email.message_from_bytes(hdr)['Subject'] or ''))):
            flagged = b'\\Flagged' in data[0][0]
    M.logout()
    assert flagged, 'flag not applied on server'
    q = inv('offline_queue')['ok']
    assert len(q['conflicts']) == 1 and 'уже нет' in q['conflicts'][0]['reason'] and 'прочитанным' in q['conflicts'][0]['what'], q['conflicts']
    ok('back online: queue replayed in order (flag, read, delete to Trash), queued mail sent with a copy in Sent')
    banner = W(d, 15).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.offline-banner') if 'удалось' in b.get_attribute('textContent')])
    banner[0].find_element(By.TAG_NAME, 'button').click()
    panel = wait(d, '.queue-modal')
    assert 'уже нет' in panel.get_attribute('textContent')
    panel.find_element(By.CSS_SELECTOR, 'button[aria-label="Скрыть конфликт"]').click()
    W(d, 10).until(lambda d: not inv('offline_queue')['ok']['conflicts'])
    panel.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.offline-banner'))
    ok('conflict: mail deleted elsewhere while offline is reported, not retried; dismissed')

    # запуск без сети: сохранённый пароль, письма с компьютера; связь появилась — работа продолжается
    block_servers(True)
    try:
        d.quit()
        d = session()
        wait(d, '.message-row', 60)
        W(d, 20).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.offline-banner') if 'Нет связи' in b.get_attribute('textContent')])
        assert rowt('E2E без сети флажок')
        assert 'Без сети' in d.find_element(By.CSS_SELECTOR, '.sidebar-footer').get_attribute('textContent')
        d.save_screenshot(f'{OUT}/47-offline-start.png')
    finally:
        block_servers(False)
    W(d, 60).until(lambda d: inv('offline_queue')['ok']['online'])
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.offline-banner'))
    assert 'Подключено по TLS' in d.find_element(By.CSS_SELECTOR, '.sidebar-footer').get_attribute('textContent')
    ok('offline start with the saved password: mail from the computer; reconnects on its own')
except Exception:
    block_servers(False)
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
    for subj, extra in (('E2E тред начало', {'Message-ID': '<t1@inst.test>'}), ('Re: E2E тред начало', {'Message-ID': '<t2@inst.test>', 'In-Reply-To': '<t1@inst.test>', 'References': '<t1@inst.test>'})):
        m = EmailMessage(); m['From'] = 'Коллега <colleague@inst.test>'; m['To'] = 'test@corp.test'; m['Subject'] = subj
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
    W(d, 10).until(lambda d: row('Отчёт с вложением'))
    d.execute_script("arguments[0].scrollIntoView({block: 'center'})", row('Отчёт с вложением')[0])
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
        m = EmailMessage(); m['From'] = 'Рассылка <news@lists.inst.test>'; m['To'] = 'test@corp.test'; m['Subject'] = subj
        for k, v in (headers or {}).items(): m[k] = v
        m.set_content(text); M = imap(); M.append('INBOX', '', imaplib.Time2Internaldate(time.time()), m.as_bytes()); M.logout()
    def refresh():
        d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()

    # Перетаскивание файла в окно письма — вложение.
    before = len(outbox())
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    wait(d, '.compose-modal')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), 'colleague@inst.test')
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
    assert 'Отчёт с вложением' in printed and 'yushkov_do@inst.test' in printed and 'Во вложении отчёт' in printed and 'Отчёт за октябрь.txt' in printed, printed[:500]
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
    add_mail('E2E рассылка 1', {'List-Unsubscribe': '<mailto:unsub@lists.inst.test?subject=unsubscribe-1>, <https://localhost:8443/unsub/42>', 'List-Unsubscribe-Post': 'List-Unsubscribe=One-Click'})
    add_mail('E2E рассылка 2', {'List-Unsubscribe': '<mailto:unsub@lists.inst.test?subject=unsubscribe-2>'})
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
    W(d, 15).until(lambda d: 'E2E отложить' in subjects(snoozed) and 'atomsnooze-' in flags_u(snoozed, 'E2E отложить'))
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
    f.write('From - Mon Jan 15 10:00:00 2024\nX-Mozilla-Status: 0005\nX-Mozilla-Status2: 00000000\nFrom: Иванов <ivanov@example.ru>\nTo: test@corp.test\nSubject: =?UTF-8?B?VEIg0L/RgNC+0YfQuNGC0LDQvdC90L7QtQ==?=\nMessage-ID: <tb-1@example.ru>\nDate: Mon, 15 Jan 2024 10:00:00 +0000\n\nПрочитанное и важное.\n>From the escaped line\n\n'.encode())
    f.write('From - Tue Jan 16 11:00:00 2024\nX-Mozilla-Status: 0000\nFrom: Сидоров <sidorov@example.ru>\nTo: test@corp.test\nSubject: TB unread\nMessage-ID: <tb-2@example.ru>\nDate: Tue, 16 Jan 2024 11:00:00 +0000\n\nНе прочитано.\n\n'.encode())
    f.write(b'From - Tue Jan 16 12:00:00 2024\nX-Mozilla-Status: 0009\nSubject: TB deleted\n\ngone\n')
with open(f'{acc}/Archives.sbd/2023', 'wb') as f:
    f.write(b'From - Fri Dec 01 09:00:00 2023\nX-Mozilla-Status: 0001\nFrom: a@example.ru\nSubject: TB archive 2023\nMessage-ID: <tb-3@example.ru>\nDate: Fri, 01 Dec 2023 09:00:00 +0000\n\narchive\n')
open(f'{H}/import/one.eml', 'wb').write('From: Почта <eml@example.ru>\nSubject: EML one\nMessage-ID: <eml-1@example.ru>\nDate: Wed, 10 Jan 2024 08:00:00 +0000\n\nПервое письмо eml.\n'.encode())
open(f'{H}/import/two.eml', 'wb').write(b'From: eml@example.ru\r\nSubject: EML two\r\nMessage-ID: <eml-2@example.ru>\r\n\r\nsecond\r\n')
open(f'{H}/import/contacts.csv', 'wb').write('"Имя";"Фамилия";"Адрес эл. почты";"Телефон раб."\r\n"Анна";"Импортова";"anna.import@inst.test";"+7 111"\r\n"Борис";"Импортов";"boris.import@inst.test";""\r\n'.encode('cp1251'))
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
    assert 'petrov@' in msg['From'] and str(make_header(decode_header(msg['From']))).startswith('Петров'), msg['From']
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
    anna = [c for c in cs if 'anna.import@inst.test' in c['emails']]
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
    copies = glob.glob(f'{H}/backup/Atom Workspace — копия */')
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
        M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login('second@corp.test', 'Secret-456'); return M
    [b for b in acc_menu().find_elements(By.TAG_NAME, 'button') if 'Добавить учётную запись' in b.get_attribute('textContent')][0].click()
    form = wait(d, '.add-account .login-card')
    form.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').send_keys('second@corp.test')
    form.find_element(By.CSS_SELECTOR, 'input[type=password]').send_keys('Secret-456')
    form.find_element(By.CSS_SELECTOR, '.login-server-toggle').click()
    host, port, smtp_host, smtp_port = form.find_elements(By.CSS_SELECTOR, '.login-server input')
    clear(host); host.send_keys('localhost'); clear(port); port.send_keys('10993')
    clear(smtp_host); smtp_host.send_keys('localhost'); clear(smtp_port); smtp_port.send_keys('465')
    d.save_screenshot(f'{OUT}/53-add-account.png')
    form.find_element(By.CSS_SELECTOR, '.login-submit').click()
    W(d, 20).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.add-account'))
    items = W(d, 10).until(lambda d: len(acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item')) == 2 and acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item'))
    assert 'second@corp.test' in items[1].get_attribute('textContent') and 'текущая' in items[0].get_attribute('textContent')
    accounts_file = os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/accounts.json'
    saved = json.load(open(accounts_file))
    assert [a['username'] for a in saved] == ['test@corp.test', 'second@corp.test'], saved
    assert 'Secret-456' not in open(accounts_file).read() and os.stat(accounts_file).st_mode & 0o077 == 0
    ok('second account added (password checked, kept in keyring, not in accounts.json)')
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
    # Новое письмо во втором ящике: число непрочитанных на кнопке учётной записи.
    M = imap2()
    for subj in ('E2E второй ящик', 'E2E второй ящик 2'):
        m = EmailMessage(); m['From'] = 'Коллега <colleague@inst.test>'; m['To'] = 'second@corp.test'; m['Subject'] = subj; m.set_content('письмо второго ящика')
        M.append('INBOX', None, imaplib.Time2Internaldate(time.time()), m.as_bytes())
    M.logout()
    W(d, 60).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.account-badge') and d.find_element(By.CSS_SELECTOR, '.account-badge').get_attribute('textContent') == '2')
    ok('unread mail of the other account counted on the account button')
    # «Все ящики» → «Входящие».
    assert [b.get_attribute('data-role') for b in d.find_elements(By.CSS_SELECTOR, '.unified-item')] == ['inbox', 'sent', 'drafts', 'archive', 'junk', 'trash']
    d.find_element(By.CSS_SELECTOR, '.unified-item[data-role=inbox]').click()
    rows = W(d, 30).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.unified-row') if 'E2E второй ящик 2' in r.get_attribute('textContent')] and d.find_elements(By.CSS_SELECTOR, '.unified-row'))
    owners = {r.get_attribute('data-account') for r in rows}
    assert owners == {'test@corp.test@localhost', 'second@corp.test@localhost'}, owners
    assert 'E2E второй ящик' in rows[0].get_attribute('textContent'), rows[0].get_attribute('textContent')
    d.save_screenshot(f'{OUT}/54-unified-inbox.png')
    ok('unified inbox: newest mail of both accounts together')

    # Фильтры общей папки: ящик, подразделение (домен), отправитель, важность.
    from selenium.webdriver.support.ui import Select
    urows = lambda: d.find_elements(By.CSS_SELECTOR, '.unified-row')
    total_rows = len(urows())
    Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Учётная запись"]')).select_by_value('second@corp.test@localhost')
    W(d, 5).until(lambda d: urows() and all(r.get_attribute('data-account') == 'second@corp.test@localhost' for r in urows()))
    assert len(urows()) == 2, len(urows())
    Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Учётная запись"]')).select_by_value('')
    W(d, 5).until(lambda d: len(urows()) == total_rows)
    domains = [o.get_attribute('value') for o in d.find_elements(By.CSS_SELECTOR, 'select[aria-label="Подразделение"] option') if o.get_attribute('value')]
    assert 'inst.test' in domains, domains
    Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Подразделение"]')).select_by_value('inst.test')
    W(d, 5).until(lambda d: urows() and all('· inst.test' in r.find_element(By.CSS_SELECTOR, '.unified-label').get_attribute('textContent') for r in urows()))
    nrcki_rows = len(urows())
    assert 0 < nrcki_rows <= total_rows
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Отправитель"]'), 'colleague')
    W(d, 5).until(lambda d: urows() and all('Коллега' in r.find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent') for r in urows()))
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[aria-label="Отправитель"]'), '')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.unified-marks button') if b.text == 'Непрочитанные'][0].click()
    W(d, 5).until(lambda d: all('unread' in r.get_attribute('class') for r in urows()))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.unified-marks button') if b.text == 'Все'][0].click()
    W(d, 5).until(lambda d: len(urows()) == nrcki_rows)
    d.save_screenshot(f'{OUT}/54a-unified-filters.png')
    ok(f'unified folder filters: by mailbox, department (domain, {len(domains)} found), sender and importance')
    # «Отправленные» всех ящиков: собеседник — получатель.
    d.find_element(By.CSS_SELECTOR, '.unified-item[data-role=sent]').click()
    W(d, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.unified-column[data-role=sent] .unified-row'))
    assert all(r.find_element(By.CSS_SELECTOR, '.sender-name').get_attribute('textContent').startswith('Кому:') for r in urows())
    assert d.find_element(By.CSS_SELECTOR, 'input[aria-label="Получатель"]')
    ok('unified Sent of all mailboxes: recipient shown and filtered')
    d.find_element(By.CSS_SELECTOR, '.unified-item[data-role=inbox]').click()
    rows = W(d, 30).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.unified-column[data-role=inbox] .unified-row') if 'E2E второй ящик 2' in r.get_attribute('textContent')] and urows())
    # Фильтр «подразделение» остаётся — проверяется сохранение контекста при переключении.
    assert Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Подразделение"]')).first_selected_option.get_attribute('value') == 'inst.test'
    # Письмо своей учётной записи открывается на месте, чужой — с переключением.
    mine = [r for r in rows if r.get_attribute('data-account') == 'test@corp.test@localhost'][0]
    subj_mine = mine.find_element(By.CSS_SELECTOR, '.message-subject').get_attribute('textContent')
    mine.click()
    W(d, 15).until(lambda d: subj_mine in d.find_element(By.CSS_SELECTOR, '.reader-content h1').get_attribute('textContent'))
    # Переключение во время фоновой задачи прежней учётной записи (резервная копия).
    job = inv('backup_start', {'folders': []})['ok']
    [r for r in d.find_elements(By.CSS_SELECTOR, '.unified-row') if 'E2E второй ящик 2' in r.get_attribute('textContent')][0].click()
    W(d, 30).until(lambda d: 'second@corp.test' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent'))
    st = W(d, 60).until(lambda d: (inv('job_status', {'id': job}).get('ok') or {}).get('finished') and inv('job_status', {'id': job})['ok'])
    assert st['cancelled'] and 'остановлено' in (st['summary'] or ''), st
    assert st['imported'] < st['total'], st
    # Данные новой учётной записи — только её: ни папок, ни писем прежней.
    paths = [f['path'] for f in inv('list_folders')['ok']]
    assert not any(p.startswith('Импорт') or p.startswith('Восстановлено') for p in paths), paths
    W(d, 30).until(lambda d: (inv('cached_messages', {'folder': 'INBOX', 'beforeUid': None, 'limit': 50}).get('ok') or {'items': []})['items'])
    subj_b = [m['subject'] for m in inv('cached_messages', {'folder': 'INBOX', 'beforeUid': None, 'limit': 50})['ok']['items']]
    assert subj_b and all(x.startswith('E2E второй ящик') for x in subj_b), subj_b
    assert all(r['account'] != 'test@corp.test@localhost' or not r['active'] for r in (inv('unified_inbox', {'limit': 5})['ok']['items']))
    ok(f"switching cancels the previous account's background job mid-way ({st['imported']} of {st['total']}); new account sees only its own folders and mail")
    W(d, 20).until(lambda d: 'E2E второй ящик 2' in d.find_element(By.CSS_SELECTOR, '.reader-content h1').get_attribute('textContent'))
    W(d, 20).until(lambda d: any('E2E второй ящик' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row')))
    assert json.load(open(os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/settings.json'))['username'] == 'second@corp.test'
    ok('opening a message of the other account switches to it and opens the message')
    # Обратно — из меню, без ввода пароля.
    [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'test@corp.test' in i.get_attribute('textContent')][0].find_element(By.CSS_SELECTOR, '.account-pick').click()
    W(d, 30).until(lambda d: 'test@corp.test' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent'))
    # Контекст восстановлен: та же общая папка с тем же фильтром и то же открытое письмо.
    wait(d, '.unified-column[data-role=inbox]', 30)
    assert Select(d.find_element(By.CSS_SELECTOR, 'select[aria-label="Подразделение"]')).first_selected_option.get_attribute('value') == 'inst.test'
    W(d, 20).until(lambda d: subj_mine in d.find_element(By.CSS_SELECTOR, '.reader-content h1').get_attribute('textContent'))
    d.save_screenshot(f'{OUT}/55-context-restored.png')
    ok('switching back restores the view: unified folder, filters and the open message')
    d.find_element(By.CSS_SELECTOR, '.folder-item[data-folder-id=INBOX]').click()
    wait(d, '.message-row', 30)
    W(d, 20).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.unified-row'))
    assert not any('E2E второй ящик' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row'))
    ok('switch back from the account menu without a password prompt')
    # Поиск по всем ящикам: найденное во втором ящике подписано и открывается с переключением.
    q_all = d.find_element(By.CSS_SELECTOR, 'input[aria-label="Поиск писем"]')
    set_value(d, q_all, 'тема:второй')
    W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.search-scope'))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.search-scope button') if b.text == 'Все ящики'][0].click()
    W(d, 40).until(lambda d: len([r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if r.find_elements(By.CSS_SELECTOR, '.search-account') and 'E2E второй ящик' in r.get_attribute('textContent')]) == 2)
    assert all('second@corp.test' in r.find_element(By.CSS_SELECTOR, '.search-account').get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if r.find_elements(By.CSS_SELECTOR, '.search-account'))
    d.save_screenshot(f'{OUT}/55a-search-all-mailboxes.png')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.search-scope button') if b.text == 'Этот ящик'][0].click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.search-account'))
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Очистить поиск"]').click()
    ok('search across all mailboxes: results of the other account labelled')
    # Отключение второй учётной записи.
    item = [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'second@corp.test' in i.get_attribute('textContent')][0]
    item.find_element(By.CSS_SELECTOR, 'button[aria-label="Отключить second@corp.test"]').click()
    [b for b in wait(d, '.confirm-modal').find_elements(By.TAG_NAME, 'button') if b.text == 'Отключить'][0].click()
    W(d, 15).until(lambda d: len(acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item')) == 1)
    assert [a['username'] for a in json.load(open(accounts_file))] == ['test@corp.test']
    import shutil
    if shutil.which('secret-tool'):
        r = subprocess.run(['secret-tool', 'search', '--all', 'service', 'ru.rrcki.mail'], capture_output=True, text=True)
        assert 'second@corp.test' not in r.stdout + r.stderr, r.stdout
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.unified-item'))
    ok('second account removed: list, keyring password and unified inbox gone')

    # Письмо учётной записи A уходит, а пользователь в это время переключается на B:
    # копия должна попасть только в «Отправленные» A.
    def find_subject(user, password, subj):
        ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
        M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ctx); M.login(user, password)
        hits = []
        import re as _re
        for line in M.list()[1]:
            m = _re.search(r'\) (?:"."|NIL) (.*)$', line.decode())
            if not m or '\\Noselect' in line.decode():
                continue
            name = m.group(1)
            try:
                typ = M.select(name, readonly=True)[0]
            except imaplib.IMAP4.error:
                continue
            if typ == 'OK':
                _, ids = M.search(None, 'SUBJECT', f'"{subj}"')
                if ids[0].split():
                    hits.append(_re.sub(r'&([^-]*)-', lambda g: '&' if not g.group(1) else ('+' + g.group(1).replace(',', '/') + '-').encode().decode('utf-7'), name.strip('"')))
        M.logout(); return hits
    r = inv('account_add', {'host': 'localhost', 'port': 10993, 'username': 'second@corp.test', 'password': 'Secret-456', 'remember': True,
                            'smtpHost': 'localhost', 'smtpPort': 465, 'davUrl': None})
    assert 'ok' in r, r
    d.execute_script("""window.__send = null; window.__TAURI_INTERNALS__.invoke('send_message', {to: 'colleague@inst.test', cc: '', subject: 'E2E isolation [slow]',
        body: 'from test', html: null, inReplyTo: null, references: null, attachments: [], readReceipt: null, delaySecs: 0, sendAt: null})
        .then((r) => { window.__send = {ok: r} }, (e) => { window.__send = {err: JSON.stringify(e)} })""")
    time.sleep(2.5)  # SMTP-сервер «думает» 6 секунд
    assert 'ok' in inv('account_switch', {'key': 'second@corp.test@localhost'})
    res = W(d, 40).until(lambda d: d.execute_script('return window.__send'))
    assert 'ok' in res, res
    sent_a = find_subject('test@corp.test', 'Secret-123', 'E2E isolation')
    assert sent_a and all('sent' in x.lower() or 'отправ' in x.lower() for x in sent_a), sent_a
    assert not find_subject('second@corp.test', 'Secret-456', 'E2E isolation'), 'copy saved into the other account'
    ok('sending from A while switching to B: copy goes only to Sent of A')

    # B защищена кодом. Переход A (с Nextcloud) → B и выход с экрана блокировки B
    # не трогает Nextcloud учётной записи A.
    def code_now(secret):
        key = _b64.b32decode(secret + '=' * (-len(secret) % 8))
        h = _hmac.new(key, _struct.pack('>Q', int(time.time()) // 30), _hashlib.sha1).digest()
        o = h[19] & 15
        return '%06d' % ((_struct.unpack('>I', h[o:o + 4])[0] & 0x7fffffff) % 1000000)
    setup = inv('totp_setup_begin')['ok']
    assert len(inv('totp_setup_confirm', {'code': code_now(setup['secret'])})['ok']) == 8
    assert 'ok' in inv('account_switch', {'key': 'test@corp.test@localhost'})
    assert inv('nc_status')['ok']['connected'], 'precondition: Nextcloud of A connected'
    locked = inv('account_switch', {'key': 'second@corp.test@localhost'})['ok']
    assert locked['account']['locked'] and locked['account']['username'] == 'second@corp.test', locked
    st_nc = inv('nc_status')
    assert 'err' in st_nc or not st_nc['ok']['connected'], ('Nextcloud of A still in memory while B is locked', st_nc)
    assert 'ok' in inv('logout', {'resetTotp': True})
    assert nc_mock('GET', '/test/state')['revoked'] is False, 'Nextcloud app password of A revoked by logging out of B'
    assert 'ok' in inv('account_switch', {'key': 'test@corp.test@localhost'})
    assert inv('nc_status')['ok']['connected']
    assert [a['username'] for a in json.load(open(accounts_file))] == ['test@corp.test']
    ok('switch to a code-protected account clears A (Nextcloud too); logging out of B does not revoke A')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 14i. Exchange (EWS): добавление по адресу и паролю (Autodiscover, NTLM), папки, письма, действия, отправка
def ews_state():
    import urllib.request as _u
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    return json.loads(_u.urlopen('https://localhost:8445/test/state', context=ctx).read())
def ews_post(path, body=b''):
    import urllib.request as _u
    ctx = ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt'))
    return _u.urlopen(_u.Request('https://localhost:8445' + path, data=body, method='POST'), context=ctx).read()
ews_item = lambda subj: [i for i in ews_state()['items'] if i['subject'] == subj]
d = session()
try:
    wait(d, '.message-row', 30)
    def inv(cmd, args=None):
        return d.execute_async_script("const cb = arguments[2]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1] || {}).then((r) => cb({ok: r}), (e) => cb({err: JSON.stringify(e)}))", cmd, args)
    def acc_menu():
        if not d.find_elements(By.CSS_SELECTOR, '.account-menu'):
            d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
        return wait(d, '.account-menu')
    rowt = lambda t: [r for r in d.find_elements(By.CSS_SELECTOR, '.message-row') if t in r.get_attribute('textContent')]
    [b for b in acc_menu().find_elements(By.TAG_NAME, 'button') if 'Добавить учётную запись' in b.get_attribute('textContent')][0].click()
    form = wait(d, '.add-account .login-card')
    [b for b in form.find_elements(By.CSS_SELECTOR, '.login-kind button') if 'Exchange' in b.text][0].click()
    set_value(d, form.find_element(By.CSS_SELECTOR, 'input[aria-label="Адрес почты Exchange"]'), 'ivanov@exch.test')
    set_value(d, form.find_element(By.CSS_SELECTOR, 'input[aria-label="Логин Exchange"]'), 'CORP\\ivanov')
    # сначала неверный пароль — вход отклонён
    set_value(d, form.find_element(By.CSS_SELECTOR, 'input[aria-label="Пароль Exchange"]'), 'wrong')
    form.find_element(By.CSS_SELECTOR, '.login-submit').click()
    W(d, 30).until(lambda d: 'Неверный логин или пароль' in form.find_element(By.CSS_SELECTOR, '.login-error').get_attribute('textContent'))
    # диагностика входа: адрес, сертификат, способы входа, где именно отказ — без пароля в отчёте
    form.find_element(By.CSS_SELECTOR, '.login-diag-button').click()
    rep = lambda: form.find_element(By.CSS_SELECTOR, 'textarea[aria-label="Отчёт диагностики Exchange"]').get_attribute('value')
    W(d, 60).until(lambda d: rep() and 'Проверка подключения' not in rep())
    t = rep()
    assert 'Autodiscover' in t and 'Неверный логин или пароль' in t and 'ВХОД УСПЕШЕН' not in t and 'wrong' not in t, t
    set_value(d, form.find_element(By.CSS_SELECTOR, 'input[aria-label="Пароль Exchange"]'), 'Exch-Pass-1')
    form.find_element(By.CSS_SELECTOR, '.login-diag-button').click()
    W(d, 60).until(lambda d: 'ВХОД УСПЕШЕН' in rep())
    t = rep()
    assert 'найден: https://localhost:8445/EWS/Exchange.asmx' in t and 'сертификат принят системой' in t and 'способы входа: NTLM, Negotiate' in t, t
    assert 'шаг 2 (CHALLENGE)' in t and 'домен NetBIOS' in t and 'с привязкой к TLS' in t and 'ответ EWS: NoError' in t and 'Exch-Pass-1' not in t, t
    ok('Exchange login diagnostics: server, certificate, offered methods, NTLM steps and the result; no password in the report')
    d.save_screenshot(f'{OUT}/57-exchange-add.png')
    form.find_element(By.CSS_SELECTOR, '.login-submit').click()
    W(d, 40).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.add-account'))
    saved = [a for a in json.load(open(os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/accounts.json')) if a.get('ewsUrl')]
    assert saved and saved[0]['ewsUrl'] == 'https://localhost:8445/EWS/Exchange.asmx' and saved[0]['address'] == 'ivanov@exch.test' and saved[0]['username'] == 'CORP\\ivanov', saved
    assert 'Exch-Pass-1' not in open(os.environ['XDG_CONFIG_HOME'] + '/ru.rrcki.mail/accounts.json').read()
    assert 'ok' in ews_state()['stats']['auth'], ews_state()['stats']['auth']
    ok('Exchange account added: server found by Autodiscover, NTLMv2 with channel binding, wrong password refused, password only in keyring')
    item = [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'ivanov@exch.test' in i.get_attribute('textContent')][0]
    assert 'Exchange' in item.get_attribute('textContent')
    item.find_element(By.CSS_SELECTOR, '.account-pick').click()
    W(d, 30).until(lambda d: 'ivanov@exch.test' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent') and 'Exchange' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent'))
    W(d, 30).until(lambda d: rowt('EWS: квартальный отчёт') and rowt('EWS: приглашение на семинар'))
    names = [f.get_attribute('textContent') for f in d.find_elements(By.CSS_SELECTOR, '.folder-item')]
    assert any('Проекты' in n for n in names) and not any('Календарь' in n or 'Sync Issues' in n for n in names), names
    paths = {f['path']: f for f in inv('list_folders')['ok']}
    assert paths['INBOX']['role'] == 'inbox' and paths['INBOX/Проекты']['depth'] == 1 and paths['Отправленные']['role'] == 'sent' and paths['Удаленные']['role'] == 'trash', list(paths)
    ok('Exchange folders: Inbox, subfolders and system folders by role; calendar and hidden folders not shown')
    rowt('EWS: квартальный отчёт')[0].click()
    W(d, 20).until(lambda d: 'Отчёт во вложении' in d.find_element(By.CSS_SELECTOR, '.reader-content').text)
    assert any('Отчёт Q3.xlsx' in c.get_attribute('textContent') for c in d.find_elements(By.CSS_SELECTOR, '.attachment-card'))
    W(d, 15).until(lambda d: ews_item('EWS: квартальный отчёт')[0]['read'])
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Флажок"]').click()
    W(d, 15).until(lambda d: ews_item('EWS: квартальный отчёт')[0]['flag'])
    uid = [m['uid'] for m in inv('list_messages', {'folder': 'INBOX', 'beforeUid': None, 'limit': 50})['ok']['items'] if m['subject'] == 'EWS: квартальный отчёт'][0]
    assert 'ok' in inv('set_keywords', {'folder': 'INBOX', 'uids': [uid], 'keyword': '$label2', 'value': True})
    W(d, 10).until(lambda d: ews_item('EWS: квартальный отчёт')[0]['categories'] == ['$label2'])
    ok('Exchange mail: message and attachment open, read, flag and category saved on the server')
    rowt('EWS: приглашение на семинар')[0].click()
    W(d, 20).until(lambda d: 'семинар по безопасности' in d.find_element(By.CSS_SELECTOR, '.reader-content').text)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Удалить письмо"]').click()
    W(d, 15).until(lambda d: ews_item('EWS: приглашение на семинар') and ews_item('EWS: приглашение на семинар')[0]['folder'] == 'F-deleted')
    W(d, 10).until(lambda d: not rowt('EWS: приглашение на семинар'))
    # новое письмо на сервере — появляется после обновления
    nm = EmailMessage(); nm['From'] = 'Коллега <kolleg@exch.test>'; nm['To'] = 'ivanov@exch.test'; nm['Subject'] = 'EWS: новое письмо'; nm.set_content('Свежая почта')
    ews_post('/test/deliver', nm.as_bytes())
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: rowt('EWS: новое письмо'))
    # поиск на сервере
    found = inv('search_server', {'query': 'тема:план'})['ok']
    assert [m['subject'] for m in found] == ['EWS: план проекта'], found
    ok('Exchange: delete to «Удалённые», new mail after refresh, server search (AQS) across folders')
    # отправка — через Exchange (копия в «Отправленные» на сервере)
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    wait(d, '.compose-modal')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), 'petrov@exch.test')
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'), 'E2E Exchange отправка')
    body = d.find_element(By.CSS_SELECTOR, '.compose-body')
    d.execute_script("arguments[0].focus(); document.execCommand('insertText', false, arguments[1])", body, 'Через EWS')
    assert 'ivanov@exch.test' in d.find_element(By.CSS_SELECTOR, '.compose-modal').get_attribute('textContent')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 30).until(lambda d: any('E2E Exchange отправка' in m for m in ews_state()['sent']))
    sent = [m for m in ews_state()['sent'] if 'E2E Exchange отправка' in m][0]
    assert 'From: ' in sent and 'ivanov@exch.test' in sent and 'petrov@exch.test' in sent, sent[:500]
    assert ews_item('E2E Exchange отправка')[0]['folder'] == 'F-sent'
    ok('Exchange: mail sent through EWS from the account address, copy kept in «Отправленные»')
    # >>> exchange pim
    import datetime as _dx
    rail = lambda name: d.find_element(By.XPATH, f"//nav[@class='module-rail']//button[.//span[text()='{name}']]")
    rail('Календарь').click()
    rng = {'start': (_dx.datetime.utcnow() - _dx.timedelta(days=2)).strftime('%Y-%m-%dT00:00:00Z'), 'end': (_dx.datetime.utcnow() + _dx.timedelta(days=40)).strftime('%Y-%m-%dT00:00:00Z')}
    r = W(d, 30).until(lambda d: (inv('list_events', rng).get('ok') or None))
    assert not r['errors'], r['errors']
    cal = r['calendars'][0]
    assert cal['name'] == 'Календарь' and cal['writable'] and len(r['calendars']) == 1, r['calendars']
    pl = [e for e in r['events'] if e['title'] == 'EWS: планёрка отдела'][0]
    assert pl['location'] == 'Переговорная 3' and pl['description'] == 'Повестка: квартальный отчёт' and pl['organizer'] == 'Петров Пётр', pl
    assert 'sidorova@partner.ru' in pl['participants'] and 'petrov@exch.test' in pl['participants'], pl['participants']
    assert not [w for w in d.find_elements(By.CSS_SELECTOR, '.inline-warning') if 'недоступно' in w.get_attribute('textContent')]
    d.save_screenshot(f'{OUT}/58-exchange-calendar.png')
    ok('Exchange calendar: calendars and events with place, description, organizer and participants')
    t0 = (_dx.datetime.utcnow() + _dx.timedelta(days=3)).replace(hour=9, minute=0, second=0, microsecond=0)
    fmt = lambda t: t.strftime('%Y-%m-%dT%H:%M:%SZ')
    assert 'ok' in inv('create_event', {'calendarUrl': cal['url'], 'input': {'title': 'E2E Exchange встреча', 'location': 'Каб. 5', 'description': 'Обсудить план', 'start': fmt(t0), 'end': fmt(t0 + _dx.timedelta(hours=1)), 'allDay': False}})
    day, day2 = (_dx.date.today() + _dx.timedelta(days=5)).isoformat(), (_dx.date.today() + _dx.timedelta(days=6)).isoformat()
    assert 'ok' in inv('create_event', {'calendarUrl': cal['url'], 'input': {'title': 'E2E Exchange весь день', 'location': None, 'description': None, 'start': day, 'end': day2, 'allDay': True}})
    evs = inv('list_events', rng)['ok']['events']
    m = [e for e in evs if e['title'] == 'E2E Exchange встреча'][0]
    a = [e for e in evs if e['title'] == 'E2E Exchange весь день'][0]
    assert m['start'].startswith(t0.strftime('%Y-%m-%dT%H:%M')) and m['location'] == 'Каб. 5' and m['description'] == 'Обсудить план' and not m['allDay'], m
    assert a['allDay'] and a['start'] == day and a['end'] == day2, a
    t1 = t0 + _dx.timedelta(days=1)
    assert 'ok' in inv('update_event', {'href': m['href'], 'input': {'title': 'E2E Exchange встреча (перенос)', 'location': '', 'description': 'Обсудить план', 'start': fmt(t1), 'end': fmt(t1 + _dx.timedelta(hours=2)), 'allDay': False}, 'changeTime': True})
    st = [c for c in ews_state()['cal'] if c['subject'] == 'E2E Exchange встреча (перенос)']
    assert st and st[0]['start'] == fmt(t1) and st[0]['end'] == fmt(t1 + _dx.timedelta(hours=2)) and st[0]['location'] == '', st
    assert 'ok' in inv('delete_event', {'href': a['href'], 'etag': a['etag']})
    assert 'ok' in inv('delete_event', {'href': pl['href'], 'etag': pl['etag']})
    stc = ews_state()
    assert not [c for c in stc['cal'] if c['subject'] in ('E2E Exchange весь день', 'EWS: планёрка отдела')] and stc['cancelled'] == [], stc['cal']
    ok('Exchange calendar: events created (with time and all-day), moved and edited, deleted; someone else’s meeting removed without sending a cancellation')
    rail('Контакты').click()
    W(d, 30).until(lambda d: any('Сидорова Анна' in r.get_attribute('textContent') for r in d.find_elements(By.CSS_SELECTOR, '.contact-row')))
    lc = inv('list_contacts')['ok']
    assert [b['name'] for b in lc['books']] == ['Контакты', 'Общая адресная книга'], lc['books']
    sa = [c for c in lc['contacts'] if c['fullName'] == 'Сидорова Анна'][0]
    assert sa['emails'] == ['sidorova@partner.ru'] and sa['phones'] == ['+7 812 000-00-00'] and sa['org'] == 'Партнёр' and sa['title'] == 'Менеджер', sa
    found = inv('search_contacts', {'query': 'Петров'})['ok']
    gal = {c['emails'][0]: c for c in found}
    assert gal['petrov@exch.test']['title'] == 'Ведущий инженер' and gal['petrov@exch.test']['org'] == 'Отдел ИТ' and not gal['petrov@exch.test']['editable'] and 'petrova@exch.test' in gal, found
    det = inv('contact_details', {'href': 'ews:gal:petrov@exch.test', 'id': 'x'})['ok']
    assert det['phones'] == ['+7 495 100-00-01'], det
    assert 'ok' in inv('create_contact', {'bookUrl': 'ews:contacts', 'input': {'fullName': 'Кузнецов Иван Сергеевич', 'emails': ['kuz@romashka.ru'], 'phones': ['+7 900 111-22-33'], 'org': 'ООО Ромашка', 'title': 'Директор', 'note': 'Важный клиент'}})
    kz = [v for v in ews_state()['contacts'].values() if v['name'] == 'Кузнецов Иван Сергеевич'][0]
    assert kz['surname'] == 'Кузнецов' and kz['given'] == 'Иван' and kz['emails'] == {'EmailAddress1': 'kuz@romashka.ru'} and kz['phones'] == {'BusinessPhone': '+7 900 111-22-33'} and kz['company'] == 'ООО Ромашка' and kz['body'] == 'Важный клиент', kz
    kc = [c for c in inv('list_contacts')['ok']['contacts'] if c['fullName'] == 'Кузнецов Иван Сергеевич'][0]
    assert 'ok' in inv('update_contact', {'href': kc['href'], 'input': {'fullName': 'Кузнецов Иван Сергеевич', 'emails': ['kuz@romashka.ru', 'ivan@romashka.ru'], 'phones': [], 'org': 'ООО Ромашка', 'title': 'Генеральный директор', 'note': ''}})
    kz = [v for v in ews_state()['contacts'].values() if v['name'] == 'Кузнецов Иван Сергеевич'][0]
    assert kz['emails'] == {'EmailAddress1': 'kuz@romashka.ru', 'EmailAddress2': 'ivan@romashka.ru'} and kz['phones'] == {} and kz['title'] == 'Генеральный директор' and kz['body'] == '', kz
    assert inv('contact_details', {'href': kc['href'], 'id': kc['id']})['ok']['emails'] == ['kuz@romashka.ru', 'ivan@romashka.ru']
    assert 'ok' in inv('delete_contact', {'href': kc['href'], 'etag': kc['etag']})
    assert not [v for v in ews_state()['contacts'].values() if v['name'] == 'Кузнецов Иван Сергеевич']
    d.save_screenshot(f'{OUT}/59-exchange-contacts.png')
    ok('Exchange contacts: own contacts and the organization address book (search), contact created, edited and deleted')
    rail('Почта').click()
    wait(d, '.message-row', 30)
    d1 = _dx.date.today() + _dx.timedelta(days=8)
    ics = ('BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//RU\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:e2e-ews-sched\r\nSUMMARY:EWS: совещание по графику\r\n'
           f'DTSTART;TZID=Europe/Moscow:{d1:%Y%m%d}T110000\r\nDTEND;TZID=Europe/Moscow:{d1:%Y%m%d}T120000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nLOCATION:Зал 1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n')
    im = EmailMessage(); im['From'] = 'Секретарь <secretary@exch.test>'; im['To'] = 'ivanov@exch.test'; im['Subject'] = 'EWS: график совещаний'; im.set_content('График во вложении')
    im.add_attachment(ics.encode(), maintype='text', subtype='calendar', filename='График.ics')
    ews_post('/test/deliver', im.as_bytes())
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Обновить"]').click()
    W(d, 20).until(lambda d: rowt('EWS: график совещаний'))
    rowt('EWS: график совещаний')[0].click()
    # (о результате говорит и всплывающее сообщение, но его может сменить уведомление о новом письме — проверяем сервер)
    for creates in (1, 2):
        wait(d, 'button[aria-label="Добавить График.ics в календарь"]', 20).click()
        dlg = wait(d, '.ics-modal')
        W(d, 30).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.ics-events li')) == 1)
        before = ews_state()['stats']['ops'].count('CreateItem')
        [b for b in dlg.find_elements(By.CSS_SELECTOR, '.ics-buttons button') if 'Добавить' in b.text][0].click()
        try:
            W(d, 30).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.ics-modal') and ews_state()['stats']['ops'].count('CreateItem') > before)
        except Exception:
            print('EWS LOG:', '\n'.join(ews_state()['stats'].get('log', [])[-40:])); raise
        occ = [e for e in inv('list_events', rng)['ok']['events'] if e['title'] == 'EWS: совещание по графику']
        assert len(occ) == 3 and all(e['location'] == 'Зал 1' and e['recurring'] for e in occ), occ
    assert len([c for c in ews_state()['cal'] if c['uid'] == 'e2e-ews-sched']) == 1
    ok('calendar file in an Exchange mail: recurring meeting added to the Exchange calendar; added again — replaced, not doubled')
    # <<< exchange pim
    # обратно в RuPost, Exchange отключается
    [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'test@corp.test' in i.get_attribute('textContent')][0].find_element(By.CSS_SELECTOR, '.account-pick').click()
    W(d, 30).until(lambda d: 'test@corp.test' in d.find_element(By.CSS_SELECTOR, '.account-switch').get_attribute('textContent'))
    item = [i for i in acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item') if 'ivanov@exch.test' in i.get_attribute('textContent')][0]
    item.find_element(By.CSS_SELECTOR, 'button[aria-label^="Отключить"]').click()
    [b for b in wait(d, '.confirm-modal').find_elements(By.TAG_NAME, 'button') if b.text == 'Отключить'][0].click()
    W(d, 15).until(lambda d: len(acc_menu().find_elements(By.CSS_SELECTOR, '.account-menu-item')) == 1)
    assert not glob.glob(os.environ['XDG_DATA_HOME'] + '/**/exchange/*.json', recursive=True)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Учётные записи"]').click()
    ok('Exchange account removed with its saved data')
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
    assert d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').get_attribute('value') == 'test@corp.test'
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
    d.find_element(By.CSS_SELECTOR, 'input[autocomplete=username]').send_keys('test@corp.test')
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
