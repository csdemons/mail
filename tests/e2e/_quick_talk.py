import base64
import faulthandler, json, imaplib, ssl, subprocess, sys, time, os, glob, email, email.utils, shutil, urllib.parse
from email.header import decode_header, make_header
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.support.ui import WebDriverWait, Select
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import StaleElementReferenceException, TimeoutException
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

def set_dt(d, root, value):
    # Свои поля даты и времени (DateField/TimeField): «ГГГГ-ММ-ДДTЧЧ:ММ» — в оба поля.
    date, tm = value.split('T')
    set_value(d, root.find_element(By.CSS_SELECTOR, 'input[data-kind=date]'), date)
    set_value(d, root.find_element(By.CSS_SELECTOR, 'input[data-kind=time]'), tm)

def reader_more(d, label):
    # Редкие действия письма — в меню «Ещё» панели письма.
    d.find_element(By.CSS_SELECTOR, '.reader-toolbar button[aria-label="Ещё действия"]').click()
    menu = W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.context-menu'))
    [b for b in menu.find_elements(By.TAG_NAME, 'button') if label in b.get_attribute('textContent')][0].click()

def close_composer(d):
    # Закрыть окно письма; если в нём что-то набрано — оно спросит про черновик: удалить.
    d.find_element(By.CSS_SELECTOR, '.compose-modal .compose-header button[aria-label="Закрыть"]').click()
    time.sleep(0.4)
    for ask in d.find_elements(By.CSS_SELECTOR, '[role=alertdialog][aria-label="Сохранить черновик?"]'):
        [b for b in ask.find_elements(By.TAG_NAME, 'button') if 'Удалить черновик' in b.text][0].click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.compose-modal'))

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


def scan_pdf():
    """PDF-«скан»: картинка в JPEG 2000 (так сохраняют многие МФУ)."""
    from PIL import Image, ImageDraw
    import io
    im = Image.new('L', (400, 300), 255); dr = ImageDraw.Draw(im); dr.rectangle([40, 40, 360, 120], fill=0); dr.rectangle([40, 160, 260, 200], fill=0)
    b = io.BytesIO(); im.save(b, 'JPEG2000'); j = b.getvalue()
    stream = b'q 300 0 0 225 0 0 cm /Im1 Do Q'
    objs = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
            b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 225] /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >>',
            b'<< /Length %d >>\nstream\n' % len(stream) + stream + b'\nendstream',
            b'<< /Type /XObject /Subtype /Image /Width 400 /Height 300 /Filter /JPXDecode /Length %d >>\nstream\n' % len(j) + j + b'\nendstream']
    out = b'%PDF-1.5\n'; offs = []
    for i, o in enumerate(objs, 1):
        offs.append(len(out)); out += b'%d 0 obj\n' % i + o + b'\nendobj\n'
    x = len(out)
    out += b'xref\n0 %d\n0000000000 65535 f \n' % (len(objs) + 1) + b''.join(b'%010d 00000 n \n' % o for o in offs)
    return out + b'trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n' % (len(objs) + 1, x)


def cid_pdf():
    """PDF со шрифтом, не встроенным в документ (CID, нужна карта кодировки)."""
    import io
    from reportlab.pdfgen import canvas
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.cidfonts import UnicodeCIDFont
    pdfmetrics.registerFont(UnicodeCIDFont('HeiseiMin-W3'))
    b = io.BytesIO(); c = canvas.Canvas(b, pagesize=(300, 200)); c.setFont('HeiseiMin-W3', 40); c.drawString(20, 100, '日本語テキスト'); c.save()
    return b.getvalue()


PDF_DARK = "const c = document.querySelector('.pdf-page'); if (!c) return -1; const x = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0; for (let i = 0; i < x.length; i += 4) if (x[i] < 100 && x[i + 3] > 0) n++; return n"

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
    wait(d, '.message-row', 45)  # первый вход после сборки: сервер ещё индексирует
    # Отмена отправки проверяется отдельно (14d); в остальных проверках письма уходят сразу.
    d.execute_script("localStorage.setItem('rrcki-mail.send-prefs', JSON.stringify({undoSecs: 0}))")
    time.sleep(1)
    # Длинные списки прокручиваются в своей колонке и не уходят за край низкого окна (масштаб, ноутбук).
    d.set_window_size(1280, 600); time.sleep(1)
    # Масштаб 150% — так, как его делает Astra 1.7 (CSS zoom): высота окна делится обратно.
    d.execute_script("document.documentElement.style.zoom = '1.5'; document.documentElement.style.setProperty('--css-zoom', '1.5')"); time.sleep(1)
    geo = d.execute_script('''
      const fl = document.querySelector('.folder-list'), ml = document.querySelector('.message-list');
      // Карточка обновления лежит поверх левого нижнего угла — на время проверки убираем её.
      document.querySelectorAll('.update-card').forEach((c) => { c.style.visibility = 'hidden' });
      const seen = (el) => { const r = el.getBoundingClientRect(); const h = document.elementFromPoint(r.left + r.width / 2, r.bottom - 3); return !!h && el.contains(h) };
      const before = fl.scrollTop; fl.scrollTop = 10000; const moved = fl.scrollTop > before;
      const r = (el) => Math.round(el.getBoundingClientRect().bottom);
      return { vh: innerHeight, shell: r(document.querySelector('.app-shell')), side: r(document.querySelector('.folder-sidebar')),
               list: r(ml), flOverflow: getComputedStyle(fl).overflowY, flScroll: fl.scrollHeight > fl.clientHeight, moved,
               last: r([...fl.querySelectorAll('.folder-item')].pop()) <= r(fl) + 1, mlScroll: ml.scrollHeight > ml.clientHeight,
               footer: seen(document.querySelector('.sidebar-footer')), status: seen(document.querySelector('.status-bar') || document.querySelector('.sidebar-footer')),
               listEnd: seen(ml), _: document.querySelectorAll('.update-card').forEach((c) => { c.style.visibility = '' }) };''')
    assert geo['footer'] and geo['status'] and geo['listEnd'], geo
    assert geo['flOverflow'] == 'auto' and geo['flScroll'] and geo['moved'] and geo['last'] and geo['mlScroll'], geo
    d.execute_script("document.querySelector('.folder-list').scrollTop = 0; document.documentElement.style.zoom = ''; document.documentElement.style.setProperty('--css-zoom', '1')")
    d.set_window_size(1400, 900); time.sleep(1)
    ok('long folder and message lists scroll inside their columns; nothing is cut off in a low window')
    cut = d.execute_script("return [...document.querySelectorAll('.module-rail .module span')].filter((s) => s.scrollWidth > s.clientWidth + 1).map((s) => s.textContent)")
    assert not cut, f'rail labels cut off: {cut}'
    # Главное окно не уходит на чужие страницы (ссылка мимо обработчиков, data:, перенаправление).
    for target in ['https://phish.example/login', 'data:text/html,<h1>phish</h1>', 'file:///etc/passwd']:
        d.execute_script("location.href = arguments[0]", target); time.sleep(1.5)
        assert d.find_elements(By.CSS_SELECTOR, '.message-row') and 'phish' not in d.current_url and not d.current_url.startswith(('data:', 'file:')), (target, d.current_url)
    d.execute_script("window.open('https://phish.example/popup')"); time.sleep(1.5)
    assert len(d.window_handles) == 1, d.window_handles
    ok('main window stays on the app: foreign pages, data: and file: are not opened, no pop-up windows')
    rows = d.find_elements(By.CSS_SELECTOR, '.message-row')
    assert len(rows) == 50, len(rows); ok('first page: 50 rows')

    # Обновление: уведомление о новой версии, «Что нового», загрузка с проверкой SHA-256.
    # Значок в шапке, а не карточка поверх папок; окошко открывается по щелчку.
    pill = W(d, 40).until(lambda d: d.find_element(By.CSS_SELECTOR, '.topbar .update-pill'))
    assert not d.find_elements(By.CSS_SELECTOR, '.update-card'), 'no card over the folders until the badge is clicked'
    assert '9.9.9' in pill.get_attribute('title')
    pill.click()
    card = W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, '.update-card.update-pop'))
    side = d.find_element(By.CSS_SELECTOR, '.folder-sidebar').rect
    assert card.rect['x'] > side['x'] + side['width'], (card.rect, side)
    # «Позже» закрывает окошко, значок остаётся
    [b for b in card.find_elements(By.TAG_NAME, 'button') if b.text == 'Позже'][0].click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.update-card') and d.find_elements(By.CSS_SELECTOR, '.update-pill'))
    d.find_element(By.CSS_SELECTOR, '.update-pill').click()
    card = W(d, 10).until(lambda d: d.find_element(By.CSS_SELECTOR, '.update-card'))
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
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.update-card') and not d.find_elements(By.CSS_SELECTOR, '.update-pill'))
    ok('update badge in the header: pop-over on click, «Позже» keeps the badge, closing removes it')
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
    # Непрочитанное — одна синяя точка и жирный шрифт (без заливки строки).
    st = d.execute_script("""const r = document.querySelector('.message-row.unread');
      return { dot: getComputedStyle(r.querySelector('.unread-dot')).backgroundColor, bold: getComputedStyle(r.querySelector('.sender-name')).fontWeight }""")
    assert st['dot'] not in ('rgba(0, 0, 0, 0)', 'transparent') and int(st['bold']) >= 600, st
    time.sleep(0.5); d.save_screenshot(f'{OUT}/14-dark-inbox.png')
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    to = wait(d, '.compose-modal .recipient-field input'); to.send_keys('Иванов Иван <ivanov@inst.test>')
    assert d.execute_script("return getComputedStyle(arguments[0]).color", to) == 'rgb(244, 247, 252)'
    sel = d.find_element(By.CSS_SELECTOR, '.compose-modal .rt-font')
    assert d.execute_script("return getComputedStyle(arguments[0]).backgroundColor", sel) == 'rgb(28, 35, 48)'
    d.save_screenshot(f'{OUT}/15-dark-compose.png')
    close_composer(d)
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Тема"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.app-shell.theme-dark'))
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    to = wait(d, '.compose-modal .recipient-field input')
    assert d.execute_script("return getComputedStyle(arguments[0]).color", to) == 'rgb(15, 23, 42)'
    close_composer(d)
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
    times = d.find_elements(By.CSS_SELECTOR, '.editor-modal input[data-kind=time]')
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
    times = d.find_elements(By.CSS_SELECTOR, '.editor-modal input[data-kind=time]')
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
    assert all(not i.is_enabled() for i in d.find_elements(By.CSS_SELECTOR, '.editor-modal input[data-kind=time], .editor-modal input[data-kind=date]'))
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
    # Место в колонке: наверх той же колонки и наверх другой (над первой карточкой).
    def drag_card(title, onto, dy):
        src = [c for c in d.find_elements(By.CSS_SELECTOR, '.deck-card') if title in c.get_attribute('textContent')][0]
        ac = ActionChains(d).move_to_element(src).click_and_hold()
        for i in range(1, 5):
            ac = ac.move_by_offset(8, -6 * i)
        ac = ac.move_to_element_with_offset(onto, 0, dy).pause(0.3)
        ac.release().perform()
    col_titles = lambda i: [c.find_element(By.CSS_SELECTOR, '.deck-card-title').get_attribute('textContent').strip() for c in d.find_elements(By.CSS_SELECTOR, '.deck-column')[i].find_elements(By.CSS_SELECTOR, '.deck-card')]
    stack_order = lambda sid: [c['title'] for c in sorted([c for c in nc_mock('GET', '/test/state')['cards'] if c['stackId'] == sid and not c.get('archived')], key=lambda c: c['order'])]
    assert col_titles(1)[-1] == 'Проверить резервные копии' and len(col_titles(1)) >= 2, col_titles(1)
    top = d.find_elements(By.CSS_SELECTOR, '.deck-column')[1].find_elements(By.CSS_SELECTOR, '.deck-card')[0]
    drag_card('Проверить резервные копии', top, -top.size['height'] // 2 + 4)
    W(d, 20).until(lambda d: stack_order(11)[0] == 'Проверить резервные копии')
    W(d, 20).until(lambda d: col_titles(1)[0] == 'Проверить резервные копии')
    moved = col_titles(0)[-1]
    top = d.find_elements(By.CSS_SELECTOR, '.deck-column')[1].find_elements(By.CSS_SELECTOR, '.deck-card')[0]
    drag_card(moved, top, -top.size['height'] // 2 + 4)
    W(d, 20).until(lambda d: stack_order(11)[:2] == [moved, 'Проверить резервные копии'])
    W(d, 20).until(lambda d: col_titles(1)[:2] == [moved, 'Проверить резервные копии'] and moved not in col_titles(0))
    # и обратно в свою колонку — дальше тесты ждут её там
    drag_card(moved, d.find_elements(By.CSS_SELECTOR, '.deck-column')[0].find_element(By.TAG_NAME, 'header'), 0)
    W(d, 20).until(lambda d: moved in stack_order(10) and moved in col_titles(0))
    ok('deck: card dropped at the very top of its own column and of another column (order kept on the server)')
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
    set_value(d, panel.find_element(By.CSS_SELECTOR, 'input[data-kind=date]'), '2026-11-05')
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
    # Тема, цвет, шрифты интерфейса, писем и новых писем; «Вернуть как было».
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sec = wait(d, '.appearance-settings')
    css = lambda js: d.execute_script(js)
    def sclick(el):
        d.execute_script("arguments[0].scrollIntoView({block: 'center'})", el); time.sleep(0.2); el.click()
    sclick([b for b in sec.find_elements(By.CSS_SELECTOR, '[aria-label="Тема"] button') if b.text == 'Тёмная'][0])
    W(d, 5).until(lambda d: css("return document.documentElement.classList.contains('theme-dark')"))
    sclick([b for b in sec.find_elements(By.CSS_SELECTOR, '[aria-label="Тема"] button') if b.text == 'Светлая'][0])
    W(d, 5).until(lambda d: not css("return document.documentElement.classList.contains('theme-dark')"))
    sclick(sec.find_element(By.CSS_SELECTOR, 'button[aria-label="Цвет #2f9e44"]'))
    bg = lambda: css("const b = [...document.querySelectorAll('.primary-button')].find((x) => x.offsetParent); return b ? getComputedStyle(b).backgroundColor : ''")
    try:
        W(d, 5).until(lambda d: bg() == 'rgb(47, 158, 68)')
    except TimeoutException:
        raise AssertionError(f"accent not applied: {bg()} {css('return document.documentElement.getAttribute(\"style\")')}")
    Select(sec.find_element(By.CSS_SELECTOR, 'select[aria-label="Шрифт интерфейса"]')).select_by_visible_text('DejaVu Sans')
    W(d, 5).until(lambda d: css("return getComputedStyle(document.body).fontFamily").startswith('"DejaVu Sans"'))
    # свой шрифт по названию; опасные символы не проходят в CSS
    Select(sec.find_element(By.CSS_SELECTOR, 'select[aria-label="Шрифт писем и сообщений"]')).select_by_visible_text('Другой…')
    fi = sec.find_element(By.CSS_SELECTOR, 'input[aria-label="Шрифт писем и сообщений: название"]')
    set_value(d, fi, 'Bad"; } body { color: red'); fi.send_keys(Keys.ENTER)
    W(d, 5).until(lambda d: appearance().get('readFont') == '')
    set_value(d, fi, 'DejaVu Serif'); fi.send_keys(Keys.ENTER)
    W(d, 5).until(lambda d: appearance().get('readFont') == 'DejaVu Serif')
    assert css("return getComputedStyle(document.querySelector('.appearance-sample')).fontFamily").startswith('"DejaVu Serif"')
    Select(sec.find_element(By.CSS_SELECTOR, 'select[aria-label="Шрифт новых писем"]')).select_by_visible_text('DejaVu Sans')
    Select(sec.find_element(By.CSS_SELECTOR, 'select[aria-label="Размер шрифта новых писем"]')).select_by_value('16')
    W(d, 5).until(lambda d: appearance().get('composeFont') == 'DejaVu Sans' and appearance().get('composeSize') == 16)
    d.save_screenshot(f'{OUT}/24c-appearance-custom.png')
    d.find_element(By.CSS_SELECTOR, '.settings-modal .compose-header button[aria-label="Закрыть"]').click()
    # новое письмо: шрифт в редакторе и у получателя
    d.find_element(By.CSS_SELECTOR, '.compose-top').click()
    cm = wait(d, '.compose-modal')
    assert css("return getComputedStyle(document.querySelector('.rt-content')).fontFamily").startswith('"DejaVu Sans"')
    set_value(d, d.find_element(By.CSS_SELECTOR, '.recipient-field input'), 'ivanov@inst.test')
    set_value(d, d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'), 'E2E font mail')
    d.execute_script("arguments[0].focus(); document.execCommand('insertText', false, 'Текст своим шрифтом')", d.find_element(By.CSS_SELECTOR, '.compose-body'))
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    def sent_font():
        M = imaplib.IMAP4_SSL('localhost', 10993, ssl_context=ssl.create_default_context(cafile=os.environ.get('IMAP_TEST_CA', '/srv/imaptest/ca.crt')))
        M.login('test@corp.test', 'Secret-123'); M.select('Sent'); typ, ids = M.search(None, 'SUBJECT', '"E2E font mail"')
        raw = M.fetch(ids[0].split()[-1], '(RFC822)')[1][0][1] if ids[0] else b''
        M.logout()
        if not raw: return ''
        m = email.message_from_bytes(raw)
        return next((p.get_payload(decode=True).decode() for p in m.walk() if p.get_content_type() == 'text/html'), '')
    W(d, 30).until(lambda d: 'Текст своим шрифтом' in sent_font())
    assert "font-family: 'DejaVu Sans', Arial" in sent_font() and 'font-size: 16px' in sent_font(), sent_font()[:400]
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Настройки"]').click()
    sec = wait(d, '.appearance-settings')
    sclick([b for b in sec.find_elements(By.TAG_NAME, 'button') if 'Вернуть как было' in b.get_attribute('textContent')][0])
    W(d, 5).until(lambda d: appearance().get('uiFont') == '' and appearance().get('accent') == '' and appearance().get('composeFont') == '' and appearance().get('theme') == 'system')
    W(d, 5).until(lambda d: not css("return document.documentElement.dataset.accent"))
    d.find_element(By.CSS_SELECTOR, '.settings-modal .compose-header button[aria-label="Закрыть"]').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.appearance-settings'))
    ok('appearance: theme (light/dark/system), accent colour, interface/reading fonts (unsafe names rejected), font of new mail reaches the recipient; reset')

    # Группы досок (по проекту): вручную через меню правой кнопки; запоминается.
    board_btn = lambda title: [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item') if title in b.get_attribute('textContent')][0]
    def dmenu(title, label):
        # Список досок перерисовывается после загрузки и при группировке: элемент берём заново
        # и повторяем, если меню не открылось или пункт устарел.
        for attempt in range(4):
            try:
                el = board_btn(title)
                d.execute_script("const r = arguments[0].getBoundingClientRect(); arguments[0].dispatchEvent(new MouseEvent('contextmenu', {bubbles: true, cancelable: true, clientX: r.left + 20, clientY: r.top + 8}))", el)
                W(d, 3).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.strip() == label])
                [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.strip() == label][0].click()
                W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.ctx-menu'))
                return
            except (TimeoutException, StaleElementReferenceException, IndexError):
                if attempt == 3:
                    raise
                d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ESCAPE)
                time.sleep(0.5)
    dmenu('Объявления', 'В новую группу…')
    gi = wait(d, 'form[aria-label="Новая группа"] input')
    set_value(d, gi, 'Проект Альфа'); gi.send_keys(Keys.ENTER)
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"]'))
    dmenu('УИТ — задачи', 'В группу «Проект Альфа»')
    W(d, 5).until(lambda d: len(d.find_elements(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"] .folder-item')) == 2)
    assert not [h for h in d.find_elements(By.CSS_SELECTOR, '.board-group-head.static')], 'no "Без группы" when every board is grouped'
    d.find_element(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"] .board-group-head').click()
    W(d, 5).until(lambda d: 'Проект Альфа' in (d.execute_script("return localStorage.getItem('atom.deck.folded')") or ''))
    d.find_element(By.CSS_SELECTOR, '.board-group[data-group="Проект Альфа"] .board-group-head').click()
    dmenu('Объявления', 'Группа по названию (как было)')
    dmenu('УИТ — задачи', 'Группа по названию (как было)')
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.board-group'))
    ok('deck: boards grouped by project via right-click menu, groups fold, back to automatic grouping')
    # Новая доска: название, цвет, стандартные колонки — и она сразу открыта.
    d.find_element(By.CSS_SELECTOR, '.deck-view button[aria-label="Новая доска"]').click()
    nb = wait(d, 'form[aria-label="Новая доска"]')
    set_value(d, nb.find_element(By.CSS_SELECTOR, 'input[aria-label="Название доски"]'), 'Хозяйственная служба')
    nb.find_element(By.CSS_SELECTOR, 'button[aria-label="Цвет #31cc7c"]').click()
    [b for b in nb.find_elements(By.TAG_NAME, 'button') if b.text == 'Создать доску'][0].click()
    W(d, 20).until(lambda d: [b for b in nc_mock('GET', '/test/state')['boards'] if b['title'] == 'Хозяйственная служба'])
    nbd = [b for b in nc_mock('GET', '/test/state')['boards'] if b['title'] == 'Хозяйственная служба'][0]
    assert nbd['color'] == '31cc7c', nbd
    # Колонки создаются после доски, по одной — ждём все три.
    W(d, 20).until(lambda d: [s['title'] for s in nc_mock('GET', '/test/state')['stacks'].get(str(nbd['id']), [])] == ['К выполнению', 'В работе', 'Готово'])
    W(d, 20).until(lambda d: [h.text for h in d.find_elements(By.CSS_SELECTOR, '.deck-column header strong')] == ['К выполнению', 'В работе', 'Готово'])
    W(d, 5).until(lambda d: any('Хозяйственная служба' in b.get_attribute('textContent') for b in d.find_elements(By.CSS_SELECTOR, '.deck-view .folder-item.selected')))
    # Колонка — тоже своим окном ввода, а не системным prompt.
    [b for b in d.find_elements(By.CSS_SELECTOR, '.deck-view button') if 'Колонка' in b.text or b.get_attribute('aria-label') == 'Новая колонка'][0].click()
    ci = wait(d, 'form[aria-label="Новая колонка"] input')
    set_value(d, ci, 'Проверка'); ci.send_keys(Keys.ENTER)
    W(d, 20).until(lambda d: [h.text for h in d.find_elements(By.CSS_SELECTOR, '.deck-column header strong')][-1:] == ['Проверка'])
    board_btn('УИТ — задачи').click()
    W(d, 20).until(lambda d: 'Обновить RuPost' in d.find_element(By.CSS_SELECTOR, '.deck-columns').get_attribute('textContent'))
    ok('deck: new board created (name, colour, default columns) and opened; new column via the app dialog')

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
    # Любая реакция из полного набора: «＋» в быстрых реакциях.
    action(talk_msg('обновление в среду'), 'Реакция')
    d.execute_script("arguments[0].click()", wait(d, '.talk-react-picker button[aria-label="Другая реакция"]'))
    wait(d, '.emoji-picker.talk-react-full input[aria-label="Найти эмодзи"]').send_keys('ракета')
    rocket = W(d, 5).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-react-full .emoji-grid button') if b.text == '🚀'])[0]
    d.execute_script("arguments[0].click()", rocket)
    W(d, 10).until(lambda d: '🚀' in msg_state(1).get('reactionsSelf', []))
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-react-full'))
    talk_msg('обновление в среду').find_element(By.CSS_SELECTOR, '.talk-reaction.own').click()
    W(d, 10).until(lambda d: not msg_state(1).get('reactionsSelf'))
    ok('reaction from the full emoji set (search «ракета»)')

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
    mm = W(d, 15).until(lambda d: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if 'посмотри' in m.get_attribute('textContent')])[0]
    W(d, 10).until(lambda d: talk_msg('посмотри').find_elements(By.CSS_SELECTOR, '.talk-mention'))
    assert talk_msg('посмотри').find_element(By.CSS_SELECTOR, 'b').text == 'срочно'
    assert talk_msg('посмотри').find_element(By.CSS_SELECTOR, 'a').get_attribute('href') == 'https://inst.test/doc'
    ok('mention autocomplete (@"id"), bold and link rendered')

    # Эмодзи из панели.
    d.find_element(By.CSS_SELECTOR, '.talk-emoji').click()
    # Полный набор, как в Talk: все категории (больше 1500 эмодзи).
    W(d, 10).until(lambda d: d.execute_script("return document.querySelectorAll('.emoji-picker .emoji-section .emoji-grid button').length") > 1500)
    assert len(d.find_elements(By.CSS_SELECTOR, '.emoji-picker .emoji-tabs button')) == 10
    # Поиск по-русски.
    d.find_element(By.CSS_SELECTOR, '.emoji-picker input[aria-label="Найти эмодзи"]').send_keys('пицца')
    W(d, 5).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.emoji-picker .emoji-grid button') if b.text == '🍕'])[0].click()
    W(d, 5).until(lambda d: '🍕' in d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value'))
    # Оттенок кожи.
    d.find_element(By.CSS_SELECTOR, '.emoji-picker .emoji-tones button[aria-label="Оттенок 3"]').click()
    srch = d.find_element(By.CSS_SELECTOR, '.emoji-picker input[aria-label="Найти эмодзи"]'); clear(srch); srch.send_keys('большой палец вверх')
    W(d, 5).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.emoji-picker .emoji-grid button') if b.text == '👍🏽'])[0].click()
    W(d, 5).until(lambda d: '👍🏽' in d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value'))
    d.find_element(By.CSS_SELECTOR, '.emoji-picker .emoji-tones button[aria-label="Без оттенка"]').click()
    clear(d.find_element(By.CSS_SELECTOR, '.emoji-picker input[aria-label="Найти эмодзи"]'))
    # Недавние — первыми.
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.emoji-picker .emoji-section .emoji-grid button'))
    first = d.find_element(By.CSS_SELECTOR, '.emoji-picker .emoji-section .emoji-grid button').text
    assert first.replace('\ufe0f', '') == '👍', first
    d.find_element(By.CSS_SELECTOR, '.talk-emoji').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.emoji-picker'))
    d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['messages']['abc123'][-1].get('raw', '').replace('\ufe0f', '') == '🍕👍🏽')
    ok('emoji picker: full Unicode set by category, Russian search, skin tone, recent first; sends')
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
    W(d, 10).until(lambda d: [b for b in modal.find_elements(By.TAG_NAME, 'button') if 'Создать карточку' in b.text and b.is_enabled()])[0].click()
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

    # Файл, скопированный в файловом менеджере, и картинка, которых WebView в событии не отдаёт, —
    # из буфера обмена системы (X11: text/uri-list, image/png).
    if shutil.which('xclip'):
        import subprocess
        def clip(target, data):
            p = subprocess.Popen(['xclip', '-selection', 'clipboard', '-t', target, '-i'], stdin=subprocess.PIPE)
            p.communicate(data); time.sleep(0.5)
        def paste_empty(types=(), text=''):
            d.execute_script("""
              const ev = new Event('paste', { bubbles: true, cancelable: true });
              Object.defineProperty(ev, 'clipboardData', { value: { files: [], items: [], types: arguments[0], getData: (t) => t === 'text/plain' ? arguments[1] : (t === 'text/uri-list' ? arguments[1] : '') } });
              document.querySelector('.talk-compose textarea').dispatchEvent(ev);
            """, list(types), text)
        doc_path = f'{OUT}/Служебная записка.txt'
        open(doc_path, 'w').write('Прошу согласовать')
        scan_path = f'{OUT}/Скан приказа.pdf'
        open(scan_path, 'wb').write(scan_pdf())
        clip('text/uri-list', ''.join('file://' + urllib.parse.quote(p) + '\r\n' for p in (doc_path, scan_path)).encode())
        paste_empty(['text/uri-list', 'text/plain'], 'file://' + doc_path)
        W(d, 10).until(lambda d: 'Служебная записка.txt' in d.find_element(By.CSS_SELECTOR, '.talk-compose .talk-pending').get_attribute('textContent') and 'Скан приказа.pdf' in d.find_element(By.CSS_SELECTOR, '.talk-compose .talk-pending').get_attribute('textContent'))
        assert d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == ''  # путь не вставлен текстом
        d.find_element(By.CSS_SELECTOR, '.talk-compose button[type=submit]').click()
        W(d, 20).until(lambda d: '/Talk/Служебная записка.txt' in nc_mock('GET', '/test/state')['files'] and '/Talk/Скан приказа.pdf' in nc_mock('GET', '/test/state')['files'])
        W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-pending'))
        # PDF из чата открывается в просмотре, а не белым листом (скан в JPEG 2000).
        chip = W(d, 20).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-messages button.talk-file, button.talk-file') if 'Скан приказа.pdf' in b.get_attribute('textContent')])[-1]
        chip.click()
        W(d, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop') and d.execute_script(PDF_DARK) > 1000)
        d.save_screenshot(f'{OUT}/talk-pdf.png')
        d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ESCAPE)
        W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.viewer-backdrop'))
        png = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEUlEQVR4nGP4z8DwH4QZYAwAR8oH+WdZbrcAAAAASUVORK5CYII=')
        clip('image/png', png)
        paste_empty()
        W(d, 10).until(lambda d: d.execute_script("const i = document.querySelector('.talk-compose .talk-pending .talk-paste-preview'); return i && i.complete && i.naturalWidth === 2"))
        d.find_element(By.CSS_SELECTOR, '.talk-compose button[type=submit]').click()
        W(d, 20).until(lambda d: len([k for k in nc_mock('GET', '/test/state')['files'] if k.startswith('/Talk/Снимок ') and k.endswith('.png')]) >= 2)
        # обычный текст по-прежнему вставляется как текст
        clip('text/plain', 'просто текст'.encode())
        paste_empty(['text/plain'], 'просто текст')
        time.sleep(1)
        assert not d.find_elements(By.CSS_SELECTOR, '.talk-pending')
        ok('chat: Ctrl+V of a file copied in the file manager and of an image from the system clipboard attaches them; text paste unchanged')

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

    # Двойной Enter, пока сервер отвечает, — одно сообщение, а не два.
    nc_mock('POST', '/test/set?slow_send=1.2')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('Один раз, а не два')
    box.send_keys(Keys.ENTER); box.send_keys(Keys.ENTER); box.send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if 'Один раз, а не два' in m.get_attribute('textContent')])
    time.sleep(2.5)
    nc_mock('POST', '/test/set?slow_send=0')
    assert len([m for m in nc_mock('GET', '/test/state')['messages']['abc123'] if m.get('raw') == 'Один раз, а не два']) == 1
    ok('chat: pressing Enter several times while sending sends the message once')

    # Закрепить сообщение на 7 дней — полоска вверху беседы; переход к сообщению; открепить.
    ctx(talk_msg('Один раз, а не два')); ctx_pick('Закрепить…')
    pd = wait(d, 'form[aria-label="Закрепить сообщение"]')
    [l for l in pd.find_elements(By.TAG_NAME, 'label') if 'На 7 дней' in l.text][0].click()
    [b for b in pd.find_elements(By.TAG_NAME, 'button') if b.text == 'Закрепить'][0].click()
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['pins'])
    pin_rec = nc_mock('GET', '/test/state')['pins'][-1]
    assert abs(pin_rec['until'] - (time.time() + 7 * 86400)) < 120, pin_rec
    bar = W(d, 20).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-pinned'))[0]
    assert 'Один раз, а не два' in bar.text and 'до ' in bar.text, bar.text
    bar.find_element(By.CSS_SELECTOR, '.talk-pinned-text').click()
    W(d, 5).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-msg.flash'))
    # навсегда — тоже можно; открепить из меню сообщения
    ctx(talk_msg('Один раз, а не два')); ctx_pick('Открепить')
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['unpins'] and not d.find_elements(By.CSS_SELECTOR, '.talk-pinned'))
    ok('chat: pin a message for 7 days (bar on top, jump to it), unpin')

    # Напоминание о сообщении: «Через час»; потом убрать.
    ctx(talk_msg('Один раз, а не два')); ctx_pick('Напомнить…')
    rd = wait(d, '[aria-label="Напомнить о сообщении"]')
    [b for b in rd.find_elements(By.TAG_NAME, 'button') if b.text.startswith('Через час')][0].click()
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['reminders'])
    when = list(nc_mock('GET', '/test/state')['reminders'].values())[0]
    assert abs(when - (time.time() + 3600)) < 120, when
    ctx(talk_msg('Один раз, а не два')); ctx_pick('Напомнить…')
    rd = wait(d, '[aria-label="Напомнить о сообщении"]')
    W(d, 10).until(lambda d: 'Сейчас:' in rd.text)
    [b for b in rd.find_elements(By.TAG_NAME, 'button') if b.text == 'Убрать напоминание'][0].click()
    W(d, 15).until(lambda d: not nc_mock('GET', '/test/state')['reminders'])
    ok('chat: remind me about a message (in 1 hour), see and remove the reminder')

    # Сотрудник: профиль, доступность по календарю, письмо, личный чат — из меню автора.
    author = [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-msg button.talk-author') if 'Иванов Иван' in b.get_attribute('textContent')][-1]
    d.execute_script("arguments[0].scrollIntoView({block: 'center'})", author); time.sleep(0.6); d.execute_script("arguments[0].click()", author)
    menu_items = lambda: [b.text for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item')]
    W(d, 5).until(lambda d: {'Чат с пользователем', 'Профиль', 'Доступность (календарь)', 'Написать письмо'} <= set(menu_items()))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text == 'Доступность (календарь)'][0].click()
    pf = wait(d, '.talk-profile')
    W(d, 15).until(lambda d: 'ivanov@inst.test' in pf.text and 'Ведущий инженер' in pf.text and 'НИЦ КИ, УИТ' in pf.text and 'отошёл' in pf.text)
    W(d, 20).until(lambda d: pf.find_elements(By.CSS_SELECTOR, '.free-busy-slot'))
    assert len(pf.find_elements(By.CSS_SELECTOR, '.free-busy-slot')) == 2 and pf.find_elements(By.CSS_SELECTOR, '.free-busy-slot.tentative')
    fb = nc_mock('GET', '/test/state')['freebusy'][-1]
    assert 'ATTENDEE:mailto:ivanov@inst.test' in fb and 'ORGANIZER:mailto:' in fb and 'METHOD:REQUEST' in fb, fb
    d.save_screenshot(f'{OUT}/talk-profile-busy.png')
    [b for b in pf.find_elements(By.TAG_NAME, 'button') if 'Написать письмо' in b.text][0].click()
    cm = wait(d, '.compose-modal')
    W(d, 5).until(lambda d: 'ivanov@inst.test' in (cm.get_attribute('textContent') + ' '.join(i.get_attribute('value') or '' for i in cm.find_elements(By.CSS_SELECTOR, '.recipient-field input'))))
    close_composer(d)
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    author = W(d, 15).until(lambda d: [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-msg button.talk-author') if 'Иванов Иван' in b.get_attribute('textContent')])[-1]
    d.execute_script("arguments[0].scrollIntoView({block: 'center'})", author); time.sleep(0.6); d.execute_script("arguments[0].click()", author)
    W(d, 5).until(lambda d: 'Чат с пользователем' in menu_items())
    [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text == 'Чат с пользователем'][0].click()
    W(d, 15).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-view .calendar-title').text == 'Иванов Иван')
    # в личной беседе — кнопка профиля в шапке
    d.find_element(By.CSS_SELECTOR, 'button[aria-label="Профиль собеседника"]').click()
    W(d, 5).until(lambda d: 'Профиль' in menu_items() and 'Чат с пользователем' not in menu_items())
    d.find_element(By.TAG_NAME, 'body').send_keys(Keys.ESCAPE)
    [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if 'УИТ — общий' in r.get_attribute('textContent')][0].click()
    W(d, 15).until(lambda d: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if 'Один раз, а не два' in m.get_attribute('textContent')])
    ok('chat: colleague menu — profile with status, availability from the calendar (free/busy), write an e-mail, open a direct chat')

    # Ветки (Talk 22): новая ветка из поля ввода, ответ в панели ветки, уведомления ветки, список веток.
    room_of = lambda tok: next(r for r in nc_mock('GET', '/test/state')['rooms'] if r['token'] == tok)
    d.find_element(By.CSS_SELECTOR, '.talk-compose button.talk-thread-new').click()
    set_value(d, wait(d, '.talk-new-thread input[aria-label="Название ветки"]'), 'План обновления')
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('Начинаем ветку про обновление'); box.send_keys(Keys.ENTER)
    root = W(d, 15).until(lambda d: next((m for m in nc_mock('GET', '/test/state')['messages']['abc123'] if m.get('isThread')), None))
    assert root['threadTitle'] == 'План обновления' and root['message'] == 'Начинаем ветку про обновление', root
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-new-thread'))  # название ветки сброшено после отправки
    chip = W(d, 15).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.talk-thread-chip') if 'План обновления' in c.text])[0]
    d.execute_script("arguments[0].scrollIntoView({block: 'center'})", chip); time.sleep(0.3); chip.click()
    panel = wait(d, '.talk-thread-panel')
    W(d, 10).until(lambda d: 'План обновления' in panel.text and 'Начинаем ветку про обновление' in panel.text)
    reply = panel.find_element(By.CSS_SELECTOR, 'textarea[aria-label="Ответ в ветке"]')
    reply.send_keys('Ответ в ветке: готов помочь'); reply.send_keys(Keys.ENTER)
    W(d, 15).until(lambda d: any(m.get('threadId') == root['id'] and not m.get('isThread') and m['message'] == 'Ответ в ветке: готов помочь' for m in nc_mock('GET', '/test/state')['messages']['abc123']))
    W(d, 15).until(lambda d: 'Ответ в ветке: готов помочь' in d.find_element(By.CSS_SELECTOR, '.talk-thread-panel').text)
    W(d, 15).until(lambda d: [c for c in d.find_elements(By.CSS_SELECTOR, '.talk-thread-chip') if 'План обновления' in c.text and '1 ответ' in c.get_attribute('textContent')])
    Select(d.find_element(By.CSS_SELECTOR, '.talk-thread-panel select[aria-label="Уведомления ветки"]')).select_by_visible_text('Не уведомлять')
    W(d, 10).until(lambda d: nc_mock('GET', '/test/state')['threads']['abc123'][str(root['id'])].get('notify') == 3)
    d.save_screenshot(f'{OUT}/talk-thread.png')
    d.find_element(By.CSS_SELECTOR, '.talk-thread-panel button[aria-label="Все ветки"]').click()
    W(d, 10).until(lambda d: [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-thread-panel .talk-thread-row') if 'План обновления' in r.get_attribute('textContent') and '1 ответ' in r.get_attribute('textContent')])
    d.find_element(By.CSS_SELECTOR, '.talk-thread-panel button[aria-label="Закрыть ветки"]').click()
    # ветки — и кнопкой в шапке беседы
    d.find_element(By.CSS_SELECTOR, 'button.talk-threads-toggle').click()
    wait(d, '.talk-thread-panel .talk-thread-row')
    d.find_element(By.CSS_SELECTOR, 'button.talk-threads-toggle').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-thread-panel'))
    ok('chat threads: start a thread from the input, reply inside the thread panel, thread notifications, list of threads')

    # Отложенная отправка (Talk 23): «Завтра утром», перенос, отмена.
    box = d.find_element(By.CSS_SELECTOR, '.talk-compose textarea')
    box.send_keys('Напоминаю про планёрку')
    d.find_element(By.CSS_SELECTOR, '.talk-compose button.talk-schedule').click()
    wait(d, '.ctx-menu')
    [b for b in d.find_elements(By.CSS_SELECTOR, '.ctx-menu .ctx-item') if b.text.startswith('Завтра утром')][0].click()
    sch = W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['scheduled'].get('abc123'))
    tomorrow9 = datetime.datetime.combine(datetime.date.today() + datetime.timedelta(days=1), datetime.time(9, 0)).timestamp()
    assert sch[0]['message'] == 'Напоминаю про планёрку' and abs(sch[0]['sendAt'] - tomorrow9) < 5, sch
    assert not [m for m in nc_mock('GET', '/test/state')['messages']['abc123'] if m['message'] == 'Напоминаю про планёрку'], 'must not be sent now'
    W(d, 5).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-compose textarea').get_attribute('value') == '')
    bar = W(d, 10).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-scheduled'))[0]
    assert 'Отложено: 1' in bar.get_attribute('textContent'), bar.get_attribute('textContent')
    bar.find_element(By.CSS_SELECTOR, '.talk-scheduled-head').click()
    W(d, 5).until(lambda d: 'Напоминаю про планёрку' in d.find_element(By.CSS_SELECTOR, '.talk-scheduled').get_attribute('textContent'))
    [b for b in d.find_elements(By.CSS_SELECTOR, '.talk-scheduled button') if b.get_attribute('textContent') == 'Перенести'][0].click()
    dlg = wait(d, '[role="dialog"][aria-label="Перенести сообщение"]')
    [b for b in dlg.find_elements(By.TAG_NAME, 'button') if b.get_attribute('textContent') == 'Перенести'][0].click()
    W(d, 10).until(lambda d: 'уйдёт' in ' '.join(t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    d.find_element(By.CSS_SELECTOR, '.talk-scheduled button[aria-label="Отменить отложенное сообщение"]').click()
    W(d, 10).until(lambda d: not nc_mock('GET', '/test/state')['scheduled']['abc123'] and not d.find_elements(By.CSS_SELECTOR, '.talk-scheduled'))
    ok('chat: scheduled message (tomorrow morning) waits on the server, can be moved and cancelled')

    # Порядок в списке бесед: архив, «важная», «конфиденциальная», звонки, «Заметки для себя».
    def room_row(name):
        return [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if name in r.get_attribute('textContent')]
    ctx(room_row('Иванов Иван')[0]); ctx_pick('В архив')
    W(d, 10).until(lambda d: room_of('dm-ivanov')['isArchived'] and not room_row('Иванов Иван'))
    toggle = wait(d, '.talk-archive-toggle')
    assert 'Архив' in toggle.get_attribute('textContent') and '1' in toggle.get_attribute('textContent'), toggle.get_attribute('textContent')
    toggle.click()
    W(d, 5).until(lambda d: room_row('Иванов Иван'))
    ctx(room_row('Иванов Иван')[0]); ctx_pick('Вернуть из архива')
    W(d, 10).until(lambda d: not room_of('dm-ivanov')['isArchived'] and not d.find_elements(By.CSS_SELECTOR, '.talk-archive-toggle'))
    ctx(room_row('УИТ — общий')[0]); ctx_pick('Важная беседа')
    W(d, 10).until(lambda d: room_of('abc123')['isImportant'] and room_row('УИТ — общий')[0].find_elements(By.CSS_SELECTOR, '.room-important'))
    ctx(room_row('УИТ — общий')[0]); ctx_pick('Конфиденциальная беседа')
    W(d, 10).until(lambda d: room_of('abc123')['isSensitive'] and 'Конфиденциальная беседа' in room_row('УИТ — общий')[0].find_element(By.CSS_SELECTOR, '.message-preview').get_attribute('textContent'))
    ctx(room_row('УИТ — общий')[0]); ctx_pick('Снять отметку «Конфиденциальная»')
    ctx(room_row('УИТ — общий')[0]); ctx_pick('Снять отметку «Важная»')
    W(d, 10).until(lambda d: not room_of('abc123')['isSensitive'] and not room_of('abc123')['isImportant'])
    ctx(room_row('УИТ — общий')[0]); ctx_pick('Не сообщать о звонках')
    W(d, 10).until(lambda d: room_of('abc123')['notificationCalls'] == 0)
    ctx(room_row('УИТ — общий')[0]); ctx_pick('Сообщать о звонках')
    W(d, 10).until(lambda d: room_of('abc123')['notificationCalls'] == 1)
    d.find_element(By.CSS_SELECTOR, '.talk-rooms button[aria-label="Заметки для себя"]').click()
    W(d, 15).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-view .calendar-title').text == 'Заметки для себя')
    assert any(r['type'] == 6 for r in nc_mock('GET', '/test/state')['rooms'])
    room_row('УИТ — общий')[0].click()
    W(d, 15).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-view .calendar-title').text == 'УИТ — общий')
    ok('chat conversation list: archive and restore, important and sensitive marks, call notifications on/off, note to self')

    # Входящий звонок: карточка в любом разделе, «Ответить» открывает звонок, «Отклонить» убирает этот звонок.
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Почта']]").click()
    call_file = os.path.join(os.environ['E2E_HOME'], 'saved', 'call-window.txt')
    if os.path.exists(call_file): os.remove(call_file)
    nc_mock('POST', '/test/set?call=abc123')
    card = W(d, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.incoming-call'))[0]
    assert 'УИТ — общий' in card.get_attribute('textContent') and 'Входящий звонок' in card.get_attribute('textContent'), card.get_attribute('textContent')
    d.save_screenshot(f'{OUT}/talk-incoming-call.png')
    card.find_element(By.CSS_SELECTOR, '.incoming-call-join').click()
    W(d, 10).until(lambda d: os.path.exists(call_file) and open(call_file).read().endswith('/index.php/call/abc123'))
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.incoming-call'))
    time.sleep(12)
    assert not d.find_elements(By.CSS_SELECTOR, '.incoming-call'), 'an answered call must not ring again'
    nc_mock('POST', '/test/set?call_end=abc123')
    time.sleep(1.1)
    nc_mock('POST', '/test/set?call=dm-ivanov')
    card = W(d, 30).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.incoming-call'))[0]
    assert 'Иванов Иван' in card.get_attribute('textContent'), card.get_attribute('textContent')
    # в чате — «Присоединиться» в шапке и значок звонка в списке
    d.find_element(By.XPATH, "//nav[@class='module-rail']//button[.//span[text()='Чат']]").click()
    W(d, 15).until(lambda d: room_row('Иванов Иван'))
    room_row('Иванов Иван')[0].click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-call.ongoing') and 'Присоединиться' in d.find_element(By.CSS_SELECTOR, '.talk-call').get_attribute('textContent'))
    assert room_row('Иванов Иван')[0].find_elements(By.CSS_SELECTOR, '.room-call')
    d.find_element(By.CSS_SELECTOR, '.incoming-call-decline').click()
    W(d, 5).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.incoming-call'))
    time.sleep(12)
    assert not d.find_elements(By.CSS_SELECTOR, '.incoming-call'), 'a declined call must not ring again'
    nc_mock('POST', '/test/set?call_end=dm-ivanov')
    # звонок, в котором я уже участвую, и звонок в беседе без уведомлений о звонках — не звонят
    nc_mock('POST', '/test/set?call=dm-ivanov&in_call=1')
    time.sleep(12)
    assert not d.find_elements(By.CSS_SELECTOR, '.incoming-call'), 'no ring for a call I am already in'
    nc_mock('POST', '/test/set?call_end=dm-ivanov')
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-call.ongoing'))
    room_row('УИТ — общий')[0].click()
    W(d, 15).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-view .calendar-title').text == 'УИТ — общий')
    ok('incoming Talk call: card in any section, answer opens the call, decline silences it; no ring when already in the call')

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

    # Встреча из беседы (Talk 21): календарь Nextcloud, длительность, кого пригласить.
    room_row2 = lambda name: [r for r in d.find_elements(By.CSS_SELECTOR, '.talk-rooms .room-row') if name in r.get_attribute('textContent')]
    ctx(row()); ctx_pick('Запланировать встречу…')
    md = wait(d, '[role="dialog"][aria-label="Запланировать встречу"]')
    W(d, 10).until(lambda d: [o.text for o in md.find_elements(By.CSS_SELECTOR, 'select[aria-label="Календарь"] option')] == ['Личный', 'Рабочий'])
    W(d, 10).until(lambda d: len(md.find_elements(By.CSS_SELECTOR, '.talk-meeting-people input[type=checkbox]')) >= 2)
    [l for l in md.find_elements(By.CSS_SELECTOR, '.talk-meeting-people label') if 'Петрова Анна' in l.get_attribute('textContent')][0].find_element(By.TAG_NAME, 'input').click()
    [b for b in md.find_elements(By.TAG_NAME, 'button') if b.text == 'Запланировать'][0].click()
    mt_ = W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['meetings'])[-1]
    ivanov_att = next(m['attendeeId'] for m in nc_mock('GET', '/test/state')['members']['abc123'] if m['actorId'] == 'ivanov')
    assert mt_['token'] == 'abc123' and mt_['calendarUri'] == 'personal' and mt_['end'] - mt_['start'] == 3600 and mt_['title'] == 'УИТ — общий', mt_
    assert mt_['attendeeIds'] == [ivanov_att], mt_
    W(d, 10).until(lambda d: 'приглашения отправлены' in ' '.join(t.get_attribute('textContent') for t in d.find_elements(By.CSS_SELECTOR, '.toast')))
    ok('chat: schedule a meeting from the conversation (Nextcloud calendar, duration, chosen participants invited)')

    # Личная беседа: собеседник в отпуске (календарь Nextcloud) и ближайшие общие встречи.
    room_row2('Иванов Иван')[0].click()
    info = W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-peer-info'))[0]
    W(d, 10).until(lambda d: 'в отпуске до' in info.get_attribute('textContent') and 'Замещает: Петрова Анна' in info.get_attribute('textContent'))
    W(d, 10).until(lambda d: 'Планёрка УИТ' in d.find_element(By.CSS_SELECTOR, '.talk-peer-info').get_attribute('textContent'))
    d.save_screenshot(f'{OUT}/talk-peer-info.png')
    room_row2('УИТ — общий')[0].click()
    W(d, 10).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-peer-info'))
    ok('chat: direct chat shows that the colleague is on vacation (with substitute) and upcoming shared meetings')

    # Все напоминания одним окном: переход к сообщению и снятие.
    W(d, 15).until(lambda d: [m for m in d.find_elements(By.CSS_SELECTOR, '.talk-msg') if 'Один раз, а не два' in m.get_attribute('textContent')])
    ctx(talk_msg('Один раз, а не два')); ctx_pick('Напомнить…')
    rd = wait(d, '[aria-label="Напомнить о сообщении"]')
    [b for b in rd.find_elements(By.TAG_NAME, 'button') if b.text.startswith('Через час')][0].click()
    W(d, 15).until(lambda d: nc_mock('GET', '/test/state')['reminders'])
    room_row2('Иванов Иван')[0].click()
    d.find_element(By.CSS_SELECTOR, '.talk-rooms button[aria-label="Напоминания"]').click()
    rl = wait(d, '[role="dialog"][aria-label="Напоминания"]')
    item = W(d, 10).until(lambda d: [b for b in rl.find_elements(By.CSS_SELECTOR, '.talk-reminder-open') if 'Один раз, а не два' in b.get_attribute('textContent') and 'УИТ — общий' in b.get_attribute('textContent')])[0]
    item.click()
    W(d, 15).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-view .calendar-title').text == 'УИТ — общий' and d.find_elements(By.CSS_SELECTOR, '.talk-msg.flash'))
    d.find_element(By.CSS_SELECTOR, '.talk-rooms button[aria-label="Напоминания"]').click()
    rl = wait(d, '[role="dialog"][aria-label="Напоминания"]')
    W(d, 10).until(lambda d: rl.find_elements(By.CSS_SELECTOR, 'button[aria-label="Убрать напоминание"]'))[0].click()
    W(d, 10).until(lambda d: not nc_mock('GET', '/test/state')['reminders'])
    rl.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
    ok('chat: all reminders in one window — open the message, remove the reminder')

    # Сводка непрочитанного (ИИ сервера): пропущено ≥ 3 сообщений — «Кратко о непрочитанном».
    for n in range(4):
        nc_mock('POST', '/test/inject?token=dm-ivanov&text=' + urllib.parse.quote(f'Пропущенное сообщение {n + 1}'))
    # (первое может уже прочитать запрос ленты беседы, из которой только что ушли, — тогда их 3)
    W(d, 30).until(lambda d: room_row2('Иванов Иван') and [c for c in room_row2('Иванов Иван')[0].find_elements(By.CSS_SELECTOR, '.folder-count') if c.get_attribute('textContent') in ('3', '4')])
    time.sleep(2)
    missed_n = room_row2('Иванов Иван')[0].find_element(By.CSS_SELECTOR, '.folder-count').get_attribute('textContent')
    room_row2('Иванов Иван')[0].click()
    bar = W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-missed'))[0]
    assert f'Пропущено сообщений: {missed_n}' in bar.get_attribute('textContent'), bar.get_attribute('textContent')
    bar.find_element(By.CSS_SELECTOR, '.text-button').click()
    sd = wait(d, '[role="dialog"][aria-label="Кратко о непрочитанном"]')
    W(d, 20).until(lambda d: f'Кратко: {missed_n} сообщ.' in sd.get_attribute('textContent') and 'Пропущенное сообщение 4' in sd.get_attribute('textContent'))
    sd.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
    ok('chat: summary of unread messages by the server AI assistant (offered after 3+ missed messages)')

    # Перевод сообщения (ИИ сервера, TaskProcessing): язык можно сменить.
    ctx(talk_msg('Пропущенное сообщение 2')); ctx_pick('Перевести…')
    td = wait(d, '[role="dialog"][aria-label="Перевод сообщения"]')
    W(d, 20).until(lambda d: '[ru] Пропущенное сообщение 2' in td.find_element(By.CSS_SELECTOR, '.talk-translate-result').get_attribute('textContent'))
    Select(td.find_element(By.CSS_SELECTOR, 'select[aria-label="Язык перевода"]')).select_by_visible_text('Английский')
    W(d, 20).until(lambda d: '[en] Пропущенное сообщение 2' in td.find_element(By.CSS_SELECTOR, '.talk-translate-result').get_attribute('textContent'))
    assert any(t['type'] == 'core:text2text:translate' and t['input']['target_language'] == 'en' for t in nc_mock('GET', '/test/state')['tasks'].values())
    td.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
    room_row2('УИТ — общий')[0].click()
    W(d, 15).until(lambda d: d.find_element(By.CSS_SELECTOR, '.talk-view .calendar-title').text == 'УИТ — общий')
    ok('chat: translate a message with the server AI translator, change the target language')

    # Модерация: зал ожидания, блокировка участника и снятие блокировки.
    inv('talk_manage', {'token': 'abc123', 'action': 'add', 'value': {'id': 'sidorov', 'source': 'users'}})
    d.find_element(By.CSS_SELECTOR, '.talk-people-toggle').click()
    W(d, 15).until(lambda d: d.find_elements(By.CSS_SELECTOR, '.talk-person[data-actor="sidorov"]'))
    ctx(d.find_element(By.CSS_SELECTOR, '.talk-person[data-actor="sidorov"]')); ctx_pick('Заблокировать…'); accept_confirm(d)
    W(d, 15).until(lambda d: [b for b in nc_mock('GET', '/test/state')['bans'].get('abc123', []) if b['bannedActorId'] == 'sidorov'])
    W(d, 15).until(lambda d: not d.find_elements(By.CSS_SELECTOR, '.talk-person[data-actor="sidorov"]'))
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Настройки беседы"]').click()
    dlg = wait(d, '.talk-settings-modal')
    W(d, 10).until(lambda d: 'Сидоров Олег' in dlg.find_element(By.CSS_SELECTOR, '.talk-bans').get_attribute('textContent'))
    [b for b in dlg.find_elements(By.CSS_SELECTOR, '.talk-bans button') if b.text == 'Разблокировать'][0].click()
    W(d, 10).until(lambda d: not nc_mock('GET', '/test/state')['bans']['abc123'])
    lobby = [l for l in dlg.find_elements(By.TAG_NAME, 'label') if 'ждут, пока модератор' in l.get_attribute('textContent')][0]
    lobby.find_element(By.TAG_NAME, 'input').click()
    W(d, 10).until(lambda d: room_of('abc123')['lobbyState'] == 1)
    dlg = wait(d, '.talk-settings-modal')
    lobby = [l for l in dlg.find_elements(By.TAG_NAME, 'label') if 'ждут, пока модератор' in l.get_attribute('textContent')][0]
    W(d, 10).until(lambda d: lobby.find_element(By.TAG_NAME, 'input').is_selected())
    lobby.find_element(By.TAG_NAME, 'input').click()
    W(d, 10).until(lambda d: room_of('abc123')['lobbyState'] == 0)
    dlg.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
    d.find_element(By.CSS_SELECTOR, '.talk-people-panel button[aria-label="Закрыть список участников"]').click()
    ok('chat moderation: ban a participant (removed, listed in settings) and unban; waiting lobby on and off')

    # Комнаты для групп: создать 2, запустить, написать во все, остановить, удалить.
    ctx(row()); ctx_pick('Комнаты для групп…')
    bd = wait(d, '[role="dialog"][aria-label="Комнаты для групп"]')
    [b for b in bd.find_elements(By.TAG_NAME, 'button') if b.text == 'Создать комнаты'][0].click()
    W(d, 15).until(lambda d: len([r for r in nc_mock('GET', '/test/state')['rooms'] if r.get('objectId') == 'abc123']) == 2)
    bd = wait(d, '[role="dialog"][aria-label="Комнаты для групп"]')
    W(d, 15).until(lambda d: [b.get_attribute('textContent') for b in bd.find_elements(By.CSS_SELECTOR, '.talk-breakout-list .talk-media-row')] == ['Комната 1', 'Комната 2'])
    assert not room_row2('Комната 1'), 'breakout rooms are not shown in the main conversation list'
    [b for b in bd.find_elements(By.TAG_NAME, 'button') if b.text == 'Запустить'][0].click()
    W(d, 10).until(lambda d: room_of('abc123')['breakoutRoomStatus'] == 1)
    bd.find_element(By.CSS_SELECTOR, 'textarea[aria-label="Сообщение во все комнаты"]').send_keys('Через 5 минут возвращаемся')
    W(d, 10).until(lambda d: [b for b in bd.find_elements(By.TAG_NAME, 'button') if b.text == 'Отправить во все' and b.is_enabled()])
    [b for b in bd.find_elements(By.TAG_NAME, 'button') if b.text == 'Отправить во все'][0].click()
    W(d, 10).until(lambda d: nc_mock('GET', '/test/state')['broadcasts'] and nc_mock('GET', '/test/state')['broadcasts'][-1]['message'] == 'Через 5 минут возвращаемся')
    W(d, 10).until(lambda d: [b for b in bd.find_elements(By.TAG_NAME, 'button') if b.text == 'Остановить'])[0].click()
    W(d, 10).until(lambda d: room_of('abc123')['breakoutRoomStatus'] == 0)
    [b for b in bd.find_elements(By.TAG_NAME, 'button') if 'Удалить комнаты' in b.text][0].click(); accept_confirm(d)
    W(d, 10).until(lambda d: not [r for r in nc_mock('GET', '/test/state')['rooms'] if r.get('objectId') == 'abc123'] and room_of('abc123')['breakoutRoomMode'] == 0)
    bd.find_element(By.CSS_SELECTOR, 'button[aria-label="Закрыть"]').click()
    ok('chat breakout rooms: create, start, message all rooms, stop, remove (moderator)')
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()
print('QUICK PASSED')
