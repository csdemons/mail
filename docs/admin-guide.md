# Руководство администратора

[← К оглавлению](README.md)

- [Развёртывание](#развёртывание)
- [Политика](#политика)
- [Обновления](#обновления)
- [Exchange](#exchange)
- [Сертификаты](#сертификаты)
- [Терминальные серверы и Linux](#терминальные-серверы-и-linux)
- [Проверка релиза](#проверка-релиза)

## Развёртывание

| Система | Пакет | Как раздавать |
|---|---|---|
| Windows 10/11 | `*_x64_ru-RU.msi` | групповая политика (GPO), SCCM; ставится для всех пользователей |
| Windows 10/11 | `*_x64-setup.exe` | вручную, в том числе без прав администратора |
| Windows 8/8.1 | `*_x64-setup-windows8.exe` | нужен WebView2 Runtime 109 |
| Astra Linux SE 1.8, Debian 12 | `*_amd64-astra1.8.deb` | репозиторий организации или `apt install ./…` |
| Astra Linux SE 1.7 | `*_amd64-astra1.7.deb` | только для 1.7 (WebKitGTK 4.0) |
| РЕД ОС, ALT | `.rpm` | |
| macOS | `*_universal.dmg` | без подписи Apple — см. [SECURITY.md](../SECURITY.md#сборка-и-поставка) |

WebView2 в Windows 10/11 обычно уже есть; в изолированной сети при сборке можно включить
`bundle.windows.webviewInstallMode = offlineInstaller`. В Astra с режимом замкнутой
программной среды (ЗПС) пакеты нужно подписать ключом организации.

## Политика

Файл `policy.json` — `/etc/rrcki-mail/` (Linux), `%ProgramData%\RRCKI Mail\` (Windows). В Windows
те же параметры задаются групповой политикой в `HKLM\SOFTWARE\Policies\RRCKI Mail` (реестр
важнее файла).

```json
{
  "remoteImages": "proxy",
  "internalNetworks": ["10.0.0.0/8"],
  "rememberPassword": true,
  "allowRememberPassword": true,
  "shareLinkDays": 30,
  "localStore": true,
  "offlineDays": 30,
  "notificationContent": "sender",
  "exchangeHosts": ["организация.ru"]
}
```

| Параметр | Реестр | Значения | По умолчанию |
|---|---|---|---|
| `remoteImages` | `RemoteImages` (строка) | картинки из интернета в письмах: `direct` — напрямую (только публичные адреса), `proxy` — через системный/корпоративный прокси, `off` — не загружать | `direct` |
| `internalNetworks` | `InternalNetworks` (через запятую) | сети, куда загрузчик картинок не обращается | — |
| `rememberPassword` | `RememberPassword` (DWORD) | галочка «Запомнить пароль» при входе | 1 |
| `allowRememberPassword` | `AllowRememberPassword` (DWORD) | 0 — запоминать пароль нельзя | 1 |
| `shareLinkDays` | `ShareLinkDays` (DWORD) | срок публичных ссылок Nextcloud «Отправить ссылкой», 0 — без срока | 30 |
| `localStore` | `LocalStore` (DWORD) | 0 — ничего из почты не хранить на компьютере (и работа без сети недоступна) | 1 |
| `offlineDays` | `OfflineDays` (DWORD) | наибольший срок хранения писем на компьютере, дней | 365 |
| `notificationContent` | `NotificationContent` (строка) | содержимое уведомлений ОС: `full`, `sender` (без темы и текста), `none` | `full` |
| `exchangeHosts` | `ExchangeHosts` (через запятую) | серверы Exchange (имя или домен) вне домена почты, которые можно принять из Autodiscover | — |

**Права на файл.** В Linux `policy.json` должен принадлежать `root:root` с правами `644`.
Если файл повреждён (ошибка JSON), не читается или его может изменить не только root,
программа **не** возвращается к значениям по умолчанию, а включает строгие ограничения
(картинки выключены, в уведомлениях ничего, пароль не запоминается, ссылки — 7 дней) и
сообщает пользователю, что политика не принята.

```bash
sudo install -o root -g root -m 644 policy.json /etc/rrcki-mail/policy.json
```

Для рабочих мест с конфиденциальной перепиской разумно: `remoteImages: off`,
`notificationContent: none`, `offlineDays` 7–30.

## Обновления

Программа проверяет `latest.json` последнего релиза при запуске и раз в 6 часов. Файл
`update.json` рядом с `policy.json`:

```json
{ "disabled": false, "url": "https://mirror.организация.ru/atom-mail/latest.json" }
```

- `disabled: true` — проверка выключена (обновляете сами, например через GPO/репозиторий).
- `url` — внутреннее зеркало: скопируйте туда из релиза без изменений `latest.json`,
  `latest.json.sig` и пакеты. Подпись проверяется встроенным ключом, поэтому подменить
  обновление через зеркало нельзя.

Реестр: `UpdateDisabled` (DWORD), `UpdateUrl` (строка). Пользователь эти настройки изменить
не может.

## Exchange

- Поддерживаются Exchange 2013, 2016, 2019 и SE через EWS (`/EWS/Exchange.asmx`).
- Вход — NTLMv2 (с привязкой к TLS-каналу и подписью MIC — работает при включённой Extended
  Protection) или Basic по HTTPS, если сервер разрешает только его либо NTLM не проходит
  (например, через обратный прокси без поддержки NTLM). Kerberos пока не используется.
- **Autodiscover** пробует `https://autodiscover.<домен почты>` и `https://<домен почты>`.
  Предложенный сервер принимается, только если он в домене почты; если почтовый домен и домен
  сервера разные (почта `@nrcki.ru`, сервер в другом домене), добавьте домен сервера в
  `exchangeHosts` — иначе пользователи указывают сервер вручную.
- Каждый сеанс работает через одно своё соединение HTTPS (вход NTLM привязан к соединению);
  прокси из переменных окружения для Exchange не используется.
- Проблемы со входом — кнопка «Диагностика подключения» на экране входа (см.
  [troubleshooting](troubleshooting.md#exchange)).

## Сертификаты

Сертификаты серверов (IMAP, SMTP, EWS, CalDAV, Nextcloud) проверяются по системному хранилищу:
SChannel в Windows, `/etc/ssl/certs` в Linux, связка ключей в macOS. Если серверы используют
сертификаты внутреннего центра сертификации, его корневой сертификат должен быть установлен в ОС
(Windows — через GPO, Astra — `update-ca-certificates`).

## Терминальные серверы и Linux

- Без Secret Service (GNOME Keyring/KWallet) программа работает, но пароль спрашивается при
  каждом запуске. На терминальных серверах разблокируйте связку ключей при входе (PAM).
- Временные файлы (обновления, перетаскиваемые вложения) создаются в личном каталоге
  пользователя (0700), а не в общем `/tmp`.
- Обновление .deb/.rpm требует прав администратора (pkexec); при `update.json`
  `disabled: true` пользователи обновлений не видят.

## Проверка релиза

К каждому релизу приложены `SHA256SUMS.txt` и подпись `SHA256SUMS.txt.sig` (Ed25519), а также
SBOM (CycloneDX). Команды проверки — в [SECURITY.md](../SECURITY.md#проверка-релиза-вручную).
