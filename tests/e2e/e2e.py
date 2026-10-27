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
    d.save_screenshot(f'{OUT}/01-login-error.png')

    # 2. вход
    clear(d.find_element(By.CSS_SELECTOR, 'input[type=password]'))
    login(d, 'Secret-123')
    wait(d, '.message-row')
    time.sleep(1)
    rows = d.find_elements(By.CSS_SELECTOR, '.message-row')
    assert len(rows) == 50, len(rows); ok('first page: 50 rows')
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
    d.save_screenshot(f'{OUT}/06-contacts.png')

    # 9. письмо из карточки контакта (SMTP + копия в «Отправленные»)
    before = len(outbox())
    [b for b in d.find_elements(By.CSS_SELECTOR, '.contact-card button') if 'Написать' in b.text][0].click()
    wait(d, '.compose-modal')
    to = d.find_element(By.CSS_SELECTOR, '.recipient-field input').get_attribute('value')
    assert 'ivanov@nrcki.ru' in to, to
    subj = d.find_element(By.CSS_SELECTOR, 'input[placeholder="Тема письма"]'); subj.send_keys('Проверка отправки из RRCKI Mail')
    d.find_element(By.CSS_SELECTOR, '.compose-body').send_keys('Добрый день!\nЭто тест.')
    d.find_element(By.CSS_SELECTOR, '.compose-modal button[type=submit]').click()
    W(d, 30).until(lambda d: 'Письмо отправлено' in (d.find_elements(By.CSS_SELECTOR, '.toast') or [None])[0].get_attribute('textContent') if d.find_elements(By.CSS_SELECTOR, '.toast') else False)
    W(d, 10).until(lambda d: len(outbox()) == before + 1)
    s1, raw1 = subject_of(outbox()[-1])
    assert s1 == 'Проверка отправки из RRCKI Mail' and b'ivanov@nrcki.ru' in raw1, s1; ok('sent via SMTP, Cyrillic subject intact')
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
    cc = d.find_elements(By.CSS_SELECTOR, '.recipient-field input')[1]; cc.send_keys('Петр')
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
except Exception:
    d.save_screenshot(f'{OUT}/fail.png'); print('PAGE TEXT:', d.find_element(By.TAG_NAME,'body').text[:800]); raise
finally:
    d.quit()

# 15. перезапуск → автовход из keyring
d = session()
try:
    wait(d, '.message-row', 30); ok('auto-login from keyring after restart')
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
