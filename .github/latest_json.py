"""Собирает latest.json для встроенного обновления из файлов релиза.
Использование: python3 .github/latest_json.py <версия> <каталог-файлов> <notes.md> <owner/repo>"""
import hashlib, json, os, sys, datetime

version, folder, notes_path, repo = sys.argv[1:5]
# Имя файла → сборка (см. updater::detect_variant).
RULES = [
    ('win8', lambda n: n.endswith('-setup-windows8.exe')),
    ('win', lambda n: n.endswith('_x64-setup.exe')),
    ('astra17', lambda n: n.endswith('_amd64-astra1.7.deb')),
    ('deb', lambda n: n.endswith('_amd64-astra1.8.deb') or n.endswith('_amd64.deb')),
    ('rpm', lambda n: n.endswith('.x86_64.rpm')),
    ('appimage', lambda n: n.endswith('_amd64.AppImage')),
]
assets = {}
for root, _, files in os.walk(folder):
    for name in sorted(files):
        variant = next((v for v, rule in RULES if rule(name)), None)
        if not variant:
            continue
        path = os.path.join(root, name)
        data = open(path, 'rb').read()
        assets[variant] = {
            'name': name,
            'url': f'https://github.com/{repo}/releases/download/v{version}/{name}',
            'sha256': hashlib.sha256(data).hexdigest(),
            'size': len(data),
        }
notes = open(notes_path, encoding='utf-8').read().split('\n### Установка')[0].strip()
manifest = {'version': version, 'notes': notes, 'pub_date': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'assets': assets}
missing = {v for v, _ in RULES} - set(assets)
if missing:
    sys.exit(f'latest.json: нет файлов для сборок {sorted(missing)}')
json.dump(manifest, open(os.path.join(folder, 'latest.json'), 'w', encoding='utf-8'), ensure_ascii=False, indent=2)
print(json.dumps({v: a['name'] for v, a in assets.items()}, ensure_ascii=False, indent=2))
