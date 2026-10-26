#!/bin/bash
# E2E: sudo apt install xvfb webkit2gtk-driver gnome-keyring dbus-x11; cargo install tauri-driver; pip install selenium
# npx tauri build --debug --no-bundle
# dbus-run-session -- tests/e2e/run.sh tests/e2e/e2e.py /tmp/shots
# Xvfb + D-Bus + разблокированный GNOME Keyring + tauri-driver
export DISPLAY=:99
# Приложение должно проверять сертификаты по системному хранилищу, как у пользователей.
unset SSL_CERT_FILE SSL_CERT_DIR
Xvfb :99 -screen 0 1600x1000x24 >/dev/null 2>&1 &
XV=$!
sleep 1
# Изолированный профиль: тест не трогает настройки и связку ключей пользователя.
E2E_HOME=$(mktemp -d)
export XDG_CONFIG_HOME="$E2E_HOME/config" XDG_DATA_HOME="$E2E_HOME/data" XDG_CACHE_HOME="$E2E_HOME/cache"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
eval "$(echo -n 'kr-pass' | gnome-keyring-daemon --unlock --components=secrets 2>/dev/null)"
export GNOME_KEYRING_CONTROL
tauri-driver >/tmp/tauri-driver.log 2>&1 &
TD=$!
sleep 2
python3 "$1" "$2"; RC=$?
kill $TD $XV 2>/dev/null
rm -rf "$E2E_HOME"
exit $RC
