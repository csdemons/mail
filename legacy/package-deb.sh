#!/bin/bash
# Собирает .deb для Astra Linux SE 1.7 из legacy/target/release/rrcki-mail.
# Зависимости вычисляет dpkg-shlibdeps по самой программе (на Debian 10).
# Использование: legacy/package-deb.sh <версия> <каталог-вывода>
set -euo pipefail
VER=$1
OUT=$2
HERE=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d)
ROOT=$WORK/pkg
install -Dm755 "$HERE/legacy/target/release/rrcki-mail" "$ROOT/usr/bin/rrcki-mail"
for s in 32x32 128x128; do
  install -Dm644 "$HERE/src-tauri/icons/$s.png" "$ROOT/usr/share/icons/hicolor/$s/apps/rrcki-mail.png"
done
install -Dm644 "$HERE/src-tauri/icons/128x128@2x.png" "$ROOT/usr/share/icons/hicolor/256x256/apps/rrcki-mail.png"
mkdir -p "$ROOT/usr/share/applications"
cat > "$ROOT/usr/share/applications/rrcki-mail.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=RRCKI Mail
Comment=Корпоративный почтовый клиент
Exec=rrcki-mail
Icon=rrcki-mail
Terminal=false
Categories=Network;Email;Office;
StartupWMClass=rrcki-mail
DESKTOP

mkdir -p "$WORK/debian"
printf 'Source: rrcki-mail\n\nPackage: rrcki-mail\nArchitecture: amd64\n' > "$WORK/debian/control"
DEPS=$(cd "$WORK" && dpkg-shlibdeps -O "pkg/usr/bin/rrcki-mail" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')
echo "Depends: $DEPS"
mkdir -p "$ROOT/DEBIAN"
SIZE=$(du -sk "$ROOT/usr" | cut -f1)
cat > "$ROOT/DEBIAN/control" <<CONTROL
Package: rrcki-mail
Version: $VER~astra1.7
Architecture: amd64
Maintainer: RRCKI <noreply@rrcki.ru>
Installed-Size: $SIZE
Depends: $DEPS
Section: mail
Priority: optional
Description: RRCKI Mail — корпоративный почтовый клиент (Astra Linux SE 1.7)
 Почта (IMAP/SMTP), календарь и контакты (CalDAV/CardDAV), задачи Deck и
 чат Talk из Nextcloud. Сборка для Debian 10 / Astra 1.7 (WebKitGTK 4.0).
CONTROL
mkdir -p "$OUT"
dpkg-deb --root-owner-group --build "$ROOT" "$OUT/rrcki-mail_${VER}_amd64-astra1.7.deb"
dpkg-deb -I "$OUT/rrcki-mail_${VER}_amd64-astra1.7.deb"
rm -rf "$WORK"
