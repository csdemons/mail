#!/bin/bash
# Локальный IMAPS-сервер (Dovecot) с тестовым ящиком для разработки и E2E.
# Ubuntu/Debian/Astra, запуск от root: sudo tests/imap-server/setup.sh
# Логин: test@rrcki.ru / Secret-123, сервер localhost:10993 (TLS, тестовый CA
# добавляется в системное хранилище — как корпоративный CA для rp.nrcki.ru).
set -euo pipefail
T=${IMAP_TEST_DIR:-/srv/imaptest}
HERE=$(cd "$(dirname "$0")" && pwd)

apt-get install -y -q dovecot-imapd >/dev/null
doveadm -c "$T/dovecot.conf" stop 2>/dev/null || true
rm -rf "$T" && mkdir -p "$T/mail" && cd "$T"

openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=RRCKI Mail Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -keyout ca.key -out ca.crt 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" -keyout srv.key -out srv.csr 2>/dev/null
printf "subjectAltName=DNS:localhost\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n" > ext.cnf
openssl x509 -req -in srv.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 30 -extfile ext.cnf -out srv.crt 2>/dev/null
cp ca.crt /usr/local/share/ca-certificates/rrcki-mail-test-ca.crt && update-ca-certificates >/dev/null

id vmail >/dev/null 2>&1 || useradd -r -M -d "$T/mail" vmail
chown -R vmail "$T/mail"; chmod 755 "$T"
echo "test@rrcki.ru:{PLAIN}Secret-123:::::" > users; chmod 644 users

cat > dovecot.conf <<CONF
protocols = imap
listen = 127.0.0.1
base_dir = $T/run
ssl = required
ssl_cert = <$T/srv.crt
ssl_key = <$T/srv.key
ssl_min_protocol = TLSv1.2
auth_mechanisms = plain login
passdb {
  driver = passwd-file
  args = $T/users
}
userdb {
  driver = static
  args = uid=vmail gid=vmail home=$T/mail/%u
}
mail_location = maildir:~/Maildir
service imap-login {
  inet_listener imap {
    port = 0
  }
  inet_listener imaps {
    port = 10993
  }
}
namespace inbox {
  inbox = yes
  separator = /
  mailbox Drafts {
    special_use = \Drafts
    auto = subscribe
  }
  mailbox Sent {
    special_use = \Sent
    auto = subscribe
  }
  mailbox Trash {
    special_use = \Trash
    auto = subscribe
  }
  mailbox Junk {
    special_use = \Junk
    auto = subscribe
  }
}
log_path = $T/dovecot.log
CONF

dovecot -c "$T/dovecot.conf" </dev/null >/dev/null 2>&1
sleep 1
IMAP_TEST_CA="$T/ca.crt" python3 "$HERE/seed.py" >/dev/null
echo "IMAPS: localhost:10993  user: test@rrcki.ru  password: Secret-123"
