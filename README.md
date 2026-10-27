# RRCKI Mail

Корпоративный почтовый клиент для **Windows** и **Linux** (в т.ч. Astra Linux SE 1.8).
Стек: [Tauri 2](https://tauri.app) (Rust) + React + TypeScript.

Приложение работает напрямую с почтовым сервером по IMAP (TLS), без промежуточного
backend: один процесс на пользователя, как Outlook или Thunderbird.

## Скачать

Готовые сборки — на странице [Releases](https://github.com/csdemons/mail/releases):
`*_x64-setup.exe` / `.msi` для Windows, `.deb` для Astra Linux SE 1.8 / Debian / Ubuntu,
`.rpm` и `.AppImage`. Что изменилось — в [CHANGELOG.md](CHANGELOG.md).

Выпуск версии: поднять `version` в `src-tauri/tauri.conf.json` и `src-tauri/Cargo.toml`,
добавить раздел в `CHANGELOG.md`, затем `git tag vX.Y.Z && git push origin vX.Y.Z` —
CI соберёт пакеты и опубликует релиз.

## Что уже работает

- Вход по логину и паролю RuPost (`rp.nrcki.ru:993`, IMAPS); пароль хранится в
  **системном хранилище** — Windows Credential Manager или Secret Service
  (GNOME Keyring / KWallet) в Linux. Если хранилища нет (например, на терминальном
  сервере без GNOME Keyring), вход работает, но пароль спрашивается при каждом запуске.
- Папки с сервера, включая вложенные и кириллические (modified UTF‑7), служебные
  папки по `SPECIAL-USE` либо по имени; счётчики непрочитанных.
- Список писем постранично (по 50, новые сверху) — с сервера загружаются только
  заголовки нужной страницы.
- Чтение: MIME, кодировки (UTF‑8, windows‑1251, KOI8‑R …), HTML‑письма отображаются
  как текст (скрипты и разметка не исполняются).
- Отметка «прочитано» и флажок синхронизируются с сервером.
- Сохранение вложений через системный диалог; имена файлов очищаются от путей.
- Автоматическое переподключение, если сервер закрыл простаивающее соединение
  или прислал ответ, который клиент не смог разобрать (иначе соединение
  «тихо» рассинхронизируется и папки выглядят пустыми).
- Постраничная догрузка по UID: новые письма во время просмотра не дают пропусков и повторов.
- **Настройки → Диагностика**: отчёт о том, что сервер отвечает по текущей папке
  (CAPABILITY, LIST, STATUS, EXAMINE, SEARCH, FETCH), без пароля — для разбора проблем.

- **Отправка писем** через SMTP (по умолчанию `rp.nrcki.ru:465`, SMTPS; 587 — STARTTLS)
  тем же логином и паролем: новое письмо, ответ (In-Reply-To/References), пересылка,
  копия, вложения до 25 МБ, копия в «Отправленные» (IMAP APPEND), подсказки адресов
  из контактов.
- **Контакты** (CardDAV): адресные книги с сервера, поиск, карточка, «Написать»;
  создание, изменение, удаление. Корпоративная книга (GAL) и ресурсы — только чтение.
- **Календарь** (CalDAV): вид «Неделя», повторяющиеся события (RRULE, EXDATE, переносы),
  события на весь день, часовые пояса (в т.ч. имена Windows/Outlook), несколько календарей.
  Сервер находится автоматически: `/.well-known/caldav|carddav`, путь SOGo
  (`/SOGo/dav/<логин>/`) или адрес из настроек.
  Создание, изменение и удаление событий (двойной щелчок по сетке или «+ Событие»);
  у повторяющихся событий меняются название/место/описание серии, участники,
  напоминания и правила повтора сохраняются. Права на запись определяются по
  `current-user-privilege-set`, изменения защищены ETag (If-Match).

- **Nextcloud** (`https://next.nrcki.ru` по умолчанию): подключение через браузер
  (Login Flow v2 — работает и с SSO/2FA) или паролем приложения; пароль приложения
  хранится в системном хранилище, при отключении отзывается на сервере.
  - **Задачи → Deck**: доски пользователя (общие доски — по правам из Deck), колонки,
    карточки с метками, сроками и исполнителями; создание, правка, «выполнено»,
    перенос (перетаскиванием или выбором колонки), удаление; доски только для чтения
    без правок; «Открыть в Nextcloud» для комментариев и вложений.
  - **Чат → Talk**: беседы со счётчиками непрочитанного, история с упоминаниями и
    файлами, отправка (Enter), ответы с цитатой, новые сообщения сразу (long-poll),
    звонок открывается в браузере.

Ещё **нет**: приглашений участникам (iTIP), отправки файлов в Talk, задач (VTODO), перемещения/удаления писем,
локального кэша (SQLite), серверного поиска, HTML с оформлением, Exchange (EWS).

## Структура

```
frontend/          React + TypeScript (Vite)
src-tauri/         Rust: окно приложения и почтовое ядро
  src/imap.rs      IMAP: подключение, папки, письма, флаги, вложения
  src/smtp.rs      отправка писем (lettre)
  src/dav.rs       CalDAV/CardDAV: обнаружение, контакты, события, RRULE
  src/nextcloud.rs Nextcloud: Login Flow v2, Deck API v1.0, Talk API
  src/settings.rs  настройки и системное хранилище паролей
  src/lib.rs       команды Tauri, вызываемые из интерфейса
  examples/smoke.rs  проверка ядра против реального сервера
tests/imap-server/ локальный Dovecot с тестовым ящиком
tests/servers/     тестовые SMTPS (aiosmtpd), CalDAV/CardDAV (Radicale), имитация Nextcloud
tests/e2e/         сквозные тесты приложения (WebDriver)
```

## Разработка

Нужны Node.js 22+, Rust (stable) и системные пакеты.

**Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
(«Разработка классических приложений на C++») и WebView2 (в Windows 10/11 уже есть).

**Linux (Ubuntu 22.04+/Debian 12/Astra 1.8):**
```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libssl-dev libsecret-1-dev build-essential
```

Запуск:
```bash
npm install          # ставит и зависимости frontend
npm run dev          # окно приложения с hot reload
```

Проверить подключение к реальному серверу без интерфейса (PowerShell):
```powershell
$env:IMAP_USERNAME = 'login'
$env:IMAP_PASSWORD = Read-Host 'IMAP password' -MaskInput
cd src-tauri; cargo run --example smoke
```
Выведет `CAPABILITY` сервера, дерево папок, первую страницу «Входящих» и время загрузки.

Nextcloud (вход через браузер, доски, беседы) без интерфейса:
```powershell
$env:NC_URL = 'https://next.nrcki.ru'
cargo run --example nextcloud_check      # NC_WRITE=1 — ещё и создать/удалить тестовую карточку и сообщение
```

SMTP, календарь и контакты без интерфейса:
```powershell
$env:SMTP_HOST = 'rp.rrcki.ru'; $env:DAV_URL = 'https://rp.nrcki.ru/'
cargo run --example groupware_check -- you@nrcki.ru   # адрес — кому отправить тестовое письмо
```

## Сборка установщиков

```bash
npm run build
```
Результат в `src-tauri/target/release/bundle/`:

| ОС | Формат | Назначение |
|---|---|---|
| Windows | `.msi` | раздача через GPO |
| Windows | `-setup.exe` (NSIS) | ручная установка, в т.ч. без прав администратора |
| Linux | `.deb` | Astra Linux, Debian, Ubuntu |
| Linux | `.rpm` | РЕД ОС, ALT и др. |
| Linux | `.AppImage` | запуск без установки |

Собирать нужно на целевой ОС; это делает CI (`.github/workflows/build.yml`):
каждый push собирает Windows- и Linux-установщики, они доступны в Artifacts.

Примечания:
- Linux-сборка выполняется на Ubuntu 22.04 (glibc 2.35), поэтому совместима с
  Astra Linux SE 1.8 (Debian 12). Пакет зависит от `libwebkit2gtk-4.1-0`.
- Если в Astra включат режим замкнутой программной среды (ЗПС), бинарники придётся
  подписывать ключом организации.
- WebView2 в Windows: при отсутствии ставится загрузчиком из интернета. Для
  изолированных сетей переключите `bundle.windows.webviewInstallMode` на
  `offlineInstaller` (установщик вырастет на ~130 МБ).
- Сертификат сервера проверяется по системному хранилищу (SChannel в Windows,
  `/etc/ssl/certs` в Linux): корпоративный корневой CA должен быть установлен в ОС.

## Тесты

```bash
cd src-tauri && cargo test && cargo clippy --all-targets   # юнит-тесты и линтер
sudo tests/imap-server/setup.sh                           # тестовый IMAPS на localhost:10993
sudo tests/servers/start.sh                               # SMTPS :465 и CalDAV/CardDAV :5232 (Radicale)
IMAP_HOST=localhost IMAP_PORT=10993 IMAP_USERNAME=test@rrcki.ru \
  IMAP_PASSWORD=Secret-123 cargo run --example smoke        # ядро против Dovecot
npx tauri build --debug --no-bundle                         # затем E2E, см. tests/e2e/run.sh
```

## План

1. Локальный кэш SQLite (окно синхронизации ~7 дней, `CONDSTORE`/`QRESYNC`,
   `IDLE` для мгновенных новых писем), догрузка старых писем из кэша.
2. Перемещение, архив, удаление (`MOVE`/`UIDPLUS`), серверный поиск.
3. Создание и изменение событий, приглашения (iTIP), корпоративная адресная книга (GAL).
4. Безопасный просмотр HTML (санитайзер + изолированный iframe, блок внешних картинок).
5. Трей со счётчиком, системные уведомления, автозапуск.
6. Exchange 2016 (EWS) как второй провайдер.
