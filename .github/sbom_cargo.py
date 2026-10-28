"""SBOM (CycloneDX 1.5, JSON) по Cargo.lock: список крейтов с версиями и контрольными суммами.
Использование: python3 .github/sbom_cargo.py <Cargo.lock> <имя-компонента> <версия> > sbom.cdx.json"""
import json, sys, uuid, datetime

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    import tomli as tomllib

lock_path, name, version = sys.argv[1:4]
lock = tomllib.load(open(lock_path, 'rb'))
components = []
for p in lock.get('package', []):
    if not p.get('source'):
        continue  # свой код (рабочие крейты проекта)
    c = {
        'type': 'library',
        'name': p['name'],
        'version': p['version'],
        'purl': f"pkg:cargo/{p['name']}@{p['version']}",
    }
    if p.get('checksum'):
        c['hashes'] = [{'alg': 'SHA-256', 'content': p['checksum']}]
    components.append(c)
sbom = {
    'bomFormat': 'CycloneDX',
    'specVersion': '1.5',
    'serialNumber': f'urn:uuid:{uuid.uuid4()}',
    'version': 1,
    'metadata': {
        'timestamp': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'component': {'type': 'application', 'name': name, 'version': version},
    },
    'components': sorted(components, key=lambda c: (c['name'], c['version'])),
}
json.dump(sbom, sys.stdout, ensure_ascii=False, indent=1)
