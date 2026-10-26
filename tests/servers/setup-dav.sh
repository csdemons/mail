#!/bin/bash
# Тестовый CalDAV/CardDAV (Radicale, TLS на 5232) с календарём и адресной книгой.
# Требует tests/imap-server/setup.sh (сертификаты в /srv/imaptest).
set -euo pipefail
T=${IMAP_TEST_DIR:-/srv/imaptest}
D=$T/dav
pkill -f "radicale --config $D/config" 2>/dev/null || true
rm -rf "$D" && mkdir -p "$D/collections"
cat > "$D/config" <<CONF
[server]
hosts = 127.0.0.1:5232
ssl = True
certificate = $T/srv.crt
key = $T/srv.key
[auth]
type = htpasswd
htpasswd_filename = $D/users
htpasswd_encryption = plain
[storage]
filesystem_folder = $D/collections
[logging]
level = warning
CONF
echo "test@rrcki.ru:Secret-123" > "$D/users"
nohup python3 -m radicale --config "$D/config" >"$D/radicale.log" 2>&1 </dev/null &
sleep 2
python3 "$(dirname "$0")/seed_dav.py"
