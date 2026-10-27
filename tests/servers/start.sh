#!/bin/bash
# Поднимает тестовые SMTPS (465) и CalDAV/CardDAV (5232) рядом с IMAP из tests/imap-server/setup.sh.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
T=${IMAP_TEST_DIR:-/srv/imaptest}
pkill -f "smtp_server.py" 2>/dev/null || true
pkill -f "nextcloud_mock.py" 2>/dev/null || true
rm -rf "$T/outbox"
setsid python3 "$HERE/smtp_server.py" "$T/srv.crt" "$T/srv.key" "$T/outbox" 465 >"$T/smtp.log" 2>&1 </dev/null &
setsid python3 "$HERE/nextcloud_mock.py" "$T/srv.crt" "$T/srv.key" 8443 >"$T/nextcloud.log" 2>&1 </dev/null &
"$HERE/setup-dav.sh" >/dev/null
sleep 1
echo "SMTPS: localhost:465, CalDAV/CardDAV: https://localhost:5232/ (test@rrcki.ru / Secret-123), Nextcloud mock: https://localhost:8443/"
