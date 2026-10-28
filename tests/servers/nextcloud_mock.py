"""Имитация Nextcloud (Login Flow v2, Deck API v1.0, Talk API v4/v1) для тестов.
Форматы ответов — по официальной документации Deck и Talk.
python3 tests/servers/nextcloud_mock.py <cert> <key> [port=8443]

Тестовые ручки:
  POST /test/inject?token=<room>&text=...  — сообщение «от коллеги» (будит long-poll)
  GET  /test/state                         — состояние для проверок
"""
import json, ssl, sys, threading, time, base64, itertools
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import unquote, urlparse, parse_qs

LOGIN, APP_PASSWORD, UID = 'test@rrcki.ru', 'app-pass-123', 'test'
lock = threading.Condition()
ids = itertools.count(100)
UPDATE_BLOB = b'fake package for update test\n' * 4000

state = {
    'dirs': {'/', '/Документы'},
    'dir_ids': {},
    'shares': [],
    'files': {'/Документы/Регламент.txt': {'id': 4242, 'data': 'Регламент работ\n'.encode()},
              '/Документы/План.docx': {'id': 4243, 'data': b'PK fake docx'}},
    'direct': {},
    'polls': 0,
    'revoked': False,
    'boards': [
        {'id': 1, 'title': 'УИТ — задачи', 'color': '0082c9', 'archived': False, 'deletedAt': 0,
         'owner': {'uid': UID, 'displayname': 'Тестовый Пользователь'},
         'permissions': {'PERMISSION_READ': True, 'PERMISSION_EDIT': True, 'PERMISSION_MANAGE': True, 'PERMISSION_SHARE': True}},
        {'id': 2, 'title': 'Объявления', 'color': 'e9322d', 'archived': False, 'deletedAt': 0,
         'owner': {'uid': 'admin', 'displayname': 'Администратор'},
         'permissions': {'PERMISSION_READ': True, 'PERMISSION_EDIT': False, 'PERMISSION_MANAGE': False, 'PERMISSION_SHARE': False}},
    ],
    'stacks': {
        1: [{'id': 10, 'title': 'К выполнению', 'boardId': 1, 'order': 0, 'deletedAt': 0},
            {'id': 11, 'title': 'В работе', 'boardId': 1, 'order': 1, 'deletedAt': 0},
            {'id': 12, 'title': 'Готово', 'boardId': 1, 'order': 2, 'deletedAt': 0}],
        2: [{'id': 20, 'title': 'Новости', 'boardId': 2, 'order': 0, 'deletedAt': 0}],
    },
    'cards': [
        {'id': 1, 'stackId': 10, 'title': 'Обновить RuPost', 'description': 'Окно работ в среду\n\n- [x] Бэкап\n- [ ] Обновить пакеты\n- [ ] Проверить почту', 'type': 'plain', 'order': 0,
         'duedate': '2026-10-30T00:00:00+00:00', 'done': None, 'archived': False, 'owner': {'uid': UID, 'displayname': 'Тестовый'},
         'labels': [{'id': 1, 'title': 'Срочно', 'color': 'e9322d', 'boardId': 1}],
         'assignedUsers': [{'participant': {'uid': 'ivanov', 'displayname': 'Иванов Иван'}}], 'commentsUnread': 1, 'attachmentCount': 0},
        {'id': 2, 'stackId': 11, 'title': 'Настроить брокер X2Go', 'description': '', 'type': 'plain', 'order': 0,
         'duedate': None, 'done': None, 'archived': False, 'owner': UID, 'labels': [], 'assignedUsers': [{'participant': {'uid': UID, 'displayname': 'Тестовый Пользователь'}}], 'commentsUnread': 0},
        {'id': 3, 'stackId': 20, 'title': 'Плановые работы 1 ноября', 'description': '', 'type': 'plain', 'order': 0,
         'duedate': None, 'done': None, 'archived': False, 'owner': 'admin', 'labels': [], 'assignedUsers': [], 'commentsUnread': 0},
    ],
    'board_labels': {1: [{'id': 1, 'title': 'Срочно', 'color': 'e9322d', 'boardId': 1}, {'id': 2, 'title': 'Сеть', 'color': '317cc9', 'boardId': 1}]},
    'comments': {1: [{'id': 501, 'objectId': 1, 'message': 'Согласовано с ИБ', 'actorId': 'ivanov', 'actorType': 'users',
                      'actorDisplayName': 'Иванов Иван', 'creationDateTime': '2026-10-26T09:00:00+00:00'}]},
    'attachments': {1: [{'id': 77, 'cardId': 1, 'type': 'file', 'data': 'regl.pdf', 'deletedAt': 0,
                         'extendedData': {'filesize': 2048, 'mimetype': 'application/pdf', 'fileid': 9001, 'info': {'filename': 'Регламент', 'extension': 'pdf'}}}]},
    'rooms': [
        {'token': 'abc123', 'type': 2, 'name': 'uit', 'displayName': 'УИТ — общий', 'readOnly': 0},
        {'token': 'dm-ivanov', 'type': 1, 'name': 'ivanov', 'displayName': 'Иванов Иван', 'readOnly': 0},
    ],
    'messages': {
        'abc123': [
            {'id': 1, 'actorType': 'users', 'actorId': 'ivanov', 'actorDisplayName': 'Иванов Иван', 'timestamp': 1793000000,
             'message': 'Коллеги, {mention-user1}, обновление в среду', 'messageType': 'comment', 'systemMessage': '',
             'messageParameters': {'mention-user1': {'type': 'user', 'id': UID, 'name': 'Тестовый Пользователь'}}},
            {'id': 2, 'actorType': 'users', 'actorId': 'petrova', 'actorDisplayName': 'Петрова Анна', 'timestamp': 1793000100,
             'message': 'План: {file}', 'messageType': 'comment', 'systemMessage': '',
             'messageParameters': {'file': {'type': 'file', 'id': '42', 'name': 'План работ.xlsx'}}},
            {'id': 3, 'actorType': 'users', 'actorId': 'petrova', 'actorDisplayName': 'Петрова Анна', 'timestamp': 1793000150,
             'message': 'reaction', 'messageType': 'system', 'systemMessage': 'reaction', 'messageParameters': []},
        ],
        'dm-ivanov': [],
    },
    'read': {'abc123': 0, 'dm-ivanov': 0},
}


def push_update(token, kind, msg):
    """Как Talk: изменение сообщения приходит служебным сообщением с parent."""
    with lock:
        state['messages'][token].append({
            'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
            'timestamp': int(time.time()), 'message': kind, 'messageType': 'system', 'systemMessage': kind,
            'messageParameters': [], 'parent': dict(msg)})
        lock.notify_all()


USERS = [{'id': 'ivanov', 'label': 'Иванов Иван', 'subline': 'ivanov@nrcki.ru'},
         {'id': 'petrova', 'label': 'Петрова Анна', 'subline': 'petrova@nrcki.ru'},
         {'id': 'sidorov', 'label': 'Сидоров Олег', 'subline': 'sidorov@nrcki.ru'}]


def room_view(r):
    msgs = [m for m in state['messages'][r['token']] if m['messageType'] != 'system']
    last = msgs[-1] if msgs else None
    unread = sum(1 for m in msgs if m['id'] > state['read'][r['token']] and m['actorId'] != UID)
    return dict(r, unreadMessages=unread, unreadMention=False,
                lastActivity=last['timestamp'] if last else 0, lastMessage=last or [])


class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *a):
        pass

    def send(self, code, obj=None, headers=None):
        body = b'' if obj is None else json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        for k, v in (headers or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def ocs(self, data, code=200, headers=None):
        self.send(code, {'ocs': {'meta': {'status': 'ok', 'statuscode': code, 'message': 'OK'}, 'data': data}}, headers)

    def peek_json(self):
        b = self.body()
        self._cached_body = b
        return b

    def body(self):
        if getattr(self, '_cached_body', None) is not None:
            b, self._cached_body = self._cached_body, None
            return b
        n = int(self.headers.get('Content-Length') or 0)
        raw = self.rfile.read(n) if n else b''
        if 'json' in (self.headers.get('Content-Type') or ''):
            return json.loads(raw or b'{}')
        return {k: v[0] for k, v in parse_qs(raw.decode()).items()}

    def authed(self):
        a = self.headers.get('Authorization', '')
        ok = a.startswith('Basic ') and base64.b64decode(a[6:]).decode() == f'{LOGIN}:{APP_PASSWORD}' and not state['revoked']
        if not ok:
            self.send(401, {'message': 'Unauthorized'})
        elif self.headers.get('OCS-APIRequest') != 'true':
            self.send(400, {'message': 'missing OCS-APIRequest'})
            return False
        return ok

    def route(self, method):
        u = urlparse(self.path)
        p, q = u.path, {k: v[0] for k, v in parse_qs(u.query).items()}
        base = f'https://localhost:{self.server.server_port}'

        # --- Аватары: пользователя (PNG) и беседы Talk (SVG)
        if method == 'GET' and p.startswith('/index.php/avatar/'):
            import zlib, struct
            uid = unquote(p.split('/')[3])
            color = {'ivanov': (200, 60, 60), 'petrova': (60, 140, 200)}.get(uid, (90, 90, 90))
            raw = b''.join(b'\x00' + bytes(color) * 8 for _ in range(8))
            ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
            body = b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', 8, 8, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b'')
            state['avatars'] = state.get('avatars', 0) + 1
            self.send_response(200); self.send_header('Content-Type', 'image/png'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)
        if method == 'GET' and p.startswith('/ocs/v2.php/apps/spreed/api/v1/room/') and p.endswith('/avatar'):
            body = b'<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#0a7"/></svg>'
            self.send_response(200); self.send_header('Content-Type', 'image/svg+xml'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)

        # --- Direct Editing: страница редактора по одноразовой ссылке (без входа)
        if method == 'GET' and p.startswith('/index.php/apps/eurooffice/direct/'):
            path = state['direct'].get(p.rsplit('/', 1)[1])
            if not path:
                return self.send(404, {})
            html = f'<!doctype html><meta charset="utf-8"><title>Euro-Office</title><h1 id="doc">Редактор: {path}</h1>'.encode()
            self.send_response(200); self.send_header('Content-Type', 'text/html; charset=utf-8'); self.send_header('Content-Length', str(len(html))); self.end_headers()
            return self.wfile.write(html)

        # --- Внешняя картинка для HTML-письма (зелёный PNG 40×30)
        if method == 'GET' and p == '/e2e/remote.png':
            import zlib, struct
            raw = b''.join(b'\x00' + bytes((30, 180, 60)) * 40 for _ in range(30))
            ch = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
            body = b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', 40, 30, 8, 2, 0, 0, 0)) + ch(b'IDAT', zlib.compress(raw)) + ch(b'IEND', b'')
            state['remote_png'] = state.get('remote_png', 0) + 1
            self.send_response(200); self.send_header('Content-Type', 'image/png'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)

        # --- Login Flow v2 (без авторизации)
        if method == 'POST' and p == '/index.php/login/v2':
            return self.send(200, {'poll': {'token': 'poll-token', 'endpoint': f'{base}/login/v2/poll'},
                                   'login': f'{base}/login/v2/flow/abc'})
        if method == 'POST' and p == '/login/v2/poll':
            if self.body().get('token') != 'poll-token':
                return self.send(404)
            state['polls'] += 1
            state['revoked'] = False
            if state['polls'] < 3:  # пользователь ещё входит в браузере
                return self.send(404, [])
            return self.send(200, {'server': base, 'loginName': LOGIN, 'appPassword': APP_PASSWORD})

        # --- обновления (без авторизации, как GitHub Releases)
        if method == 'GET' and p in ('/updates/latest.json', '/updates/latest.json.sig'):
            import hashlib, subprocess, os, base64 as b64
            body = json.dumps({'version': '9.9.9', 'notes': '- Проверка обновлений\n- Ещё пункт', 'assets': {
                'test': {'name': 'rrcki-mail_9.9.9_amd64.deb', 'url': f'{base}/updates/rrcki-mail_9.9.9_amd64.deb',
                         'sha256': hashlib.sha256(UPDATE_BLOB).hexdigest(), 'size': len(UPDATE_BLOB)}}}, ensure_ascii=False).encode()
            if p.endswith('.sig'):
                # Подпись тестовым ключом (tests/servers/update-test-key.pem) — как в CI, через openssl.
                key = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'update-test-key.pem')
                import tempfile
                with tempfile.NamedTemporaryFile() as tf:
                    tf.write(body); tf.flush()
                    sig = subprocess.run(['openssl', 'pkeyutl', '-sign', '-inkey', key, '-rawin', '-in', tf.name], capture_output=True, check=True).stdout
                body = b64.b64encode(sig) + b'\n'
            self.send_response(200); self.send_header('Content-Type', 'application/octet-stream'); self.send_header('Content-Length', str(len(body))); self.end_headers()
            return self.wfile.write(body)
        if method == 'GET' and p == '/updates/rrcki-mail_9.9.9_amd64.deb':
            self.send_response(200); self.send_header('Content-Length', str(len(UPDATE_BLOB))); self.end_headers()
            self.wfile.write(UPDATE_BLOB); return

        # --- тестовые ручки
        if p == '/test/state':
            return self.send(200, {k: state[k] for k in ('polls', 'revoked', 'cards', 'stacks', 'comments', 'shares')} | {'dirs': sorted(state['dirs']), 'remote_png': state.get('remote_png', 0)} | {'messages': state['messages'],
                             'files': {k: v['data'].decode(errors='replace') for k, v in state['files'].items()}})
        if method == 'POST' and p == '/test/inject':
            with lock:
                state['messages'][q['token']].append({
                    'id': next(ids), 'actorType': 'users', 'actorId': 'ivanov', 'actorDisplayName': 'Иванов Иван',
                    'timestamp': int(time.time()), 'message': q['text'], 'messageType': 'comment', 'systemMessage': '',
                    'messageParameters': []})
                lock.notify_all()
            return self.send(200, {})

        if not self.authed():
            return
        if p.startswith('/ocs/v2.php/apps/files/api/v1/directEditing'):
            mimes = {'docx': 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
                     'xlsx': 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
                     'pptx': 'application/vnd.openxmlformats-officedocument.presentationml.presentation'}
            if method == 'GET':
                return self.ocs({'editors': {'eurooffice': {'id': 'eurooffice', 'name': 'Euro-Office', 'mimetypes': list(mimes.values()), 'optionalMimetypes': [], 'secure': False}},
                                 'creators': {f'eurooffice_{e}': {'id': f'eurooffice_{e}', 'editor': 'eurooffice', 'name': e, 'extension': e, 'templates': False, 'mimetype': m} for e, m in mimes.items()}})
            b = self.body()
            path = b.get('path', '')
            if p.endswith('/create'):
                state['files'][path] = {'id': next(ids), 'data': b'PK new ' + path.encode()}
            elif path not in state['files']:
                return self.ocs({}, 404)
            tok = f'tok{next(ids)}'
            state['direct'][tok] = path
            return self.ocs({'url': f'{base}/index.php/apps/eurooffice/direct/{tok}'})
        if method == 'GET' and p == '/ocs/v1.php/cloud/user':
            return self.ocs({'id': UID, 'displayname': 'Тестовый Пользователь', 'email': LOGIN})
        if method == 'DELETE' and p == '/ocs/v2.php/core/apppassword':
            state['revoked'] = True
            return self.ocs([])

        # --- Deck
        D = '/index.php/apps/deck/api/v1.0/boards'
        if p == D and method == 'GET':
            return self.send(200, state['boards'])
        # Комментарии (OCS) и вложения (API 1.1)
        CM = '/ocs/v2.php/apps/deck/api/v1.0/cards/'
        if p.startswith(CM) and '/comments' in p:
            rest = p[len(CM):].split('/')
            card_id = int(rest[0]); lst = state['comments'].setdefault(card_id, [])
            if method == 'GET':
                return self.ocs(list(reversed(lst)))
            if method == 'POST':
                b = self.body()
                c = {'id': next(ids), 'objectId': card_id, 'message': b['message'], 'actorId': UID, 'actorType': 'users',
                     'actorDisplayName': 'Тестовый Пользователь', 'creationDateTime': time.strftime('%Y-%m-%dT%H:%M:%S+00:00', time.gmtime())}
                lst.append(c); return self.ocs(c)
            if method == 'DELETE' and len(rest) == 3:
                state['comments'][card_id] = [c for c in lst if c['id'] != int(rest[2])]
                return self.ocs([])
        A = '/index.php/apps/deck/api/v1.1/boards/'
        if p.startswith(A) and p.endswith('/attachments') and method == 'GET':
            card_id = int(p[len(A):].split('/')[4])
            return self.send(200, state['attachments'].get(card_id, []))
        parts = p[len(D):].strip('/').split('/') if p.startswith(D + '/') else []
        if len(parts) == 1 and method == 'GET':
            b = next(x for x in state['boards'] if x['id'] == int(parts[0]))
            return self.send(200, dict(b, labels=state['board_labels'].get(b['id'], []),
                                       users=[{'uid': 'ivanov', 'displayname': 'Иванов Иван'}],
                                       acl=[{'type': 0, 'participant': {'uid': 'petrova', 'displayname': 'Петрова Анна'}}]))
        if parts:
            board = int(parts[0])
            perms = next(b for b in state['boards'] if b['id'] == board)['permissions']
            if method != 'GET' and not perms['PERMISSION_EDIT']:
                return self.send(403, {'status': 403, 'message': 'Permission denied'})
            if parts[1:] == ['stacks'] and method == 'GET':
                out = []
                for s in state['stacks'][board]:
                    cards = sorted([c for c in state['cards'] if c['stackId'] == s['id']], key=lambda c: c['order'])
                    out.append(dict(s, cards=cards))
                return self.send(200, out)
            if parts[1:] == ['stacks'] and method == 'POST':
                b = self.body()
                s = {'id': next(ids), 'title': b['title'], 'boardId': board, 'order': b.get('order', 999), 'deletedAt': 0}
                state['stacks'][board].append(s)
                return self.send(200, s)
            if len(parts) >= 4 and parts[1] == 'stacks' and parts[3] == 'cards':
                stack = int(parts[2])
                if len(parts) == 4 and method == 'POST':
                    b = self.body()
                    for f in ('title', 'type', 'order'):
                        if f not in b:
                            return self.send(400, {'message': f'{f} missing'})
                    c = {'id': next(ids), 'stackId': stack, 'title': b['title'], 'description': b.get('description', ''),
                         'type': b['type'], 'order': b['order'], 'duedate': b.get('duedate'), 'done': None, 'archived': False,
                         'owner': {'uid': UID, 'displayname': 'Тестовый'}, 'labels': [], 'assignedUsers': None, 'commentsUnread': 0}
                    state['cards'].append(c)
                    return self.send(200, c)
                card = next((c for c in state['cards'] if c['id'] == int(parts[4])), None) if len(parts) >= 5 else None
                if card is None:
                    return self.send(404, {'message': 'not found'})
                if len(parts) == 5 and method == 'GET':
                    return self.send(200, card)
                if len(parts) == 5 and method == 'PUT':
                    b = self.body()
                    for f in ('title', 'type', 'owner', 'order', 'description'):
                        if f not in b:
                            return self.send(400, {'message': f'{f} missing'})
                    card.update(title=b['title'], description=b['description'], duedate=b.get('duedate'), done=b.get('done'))
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] in ('assignUser', 'unassignUser') and method == 'PUT':
                    uid = self.body()['userId']
                    users = card.get('assignedUsers') or []
                    users = [u for u in users if u['participant']['uid'] != uid]
                    if parts[5] == 'assignUser':
                        name = {'ivanov': 'Иванов Иван', 'petrova': 'Петрова Анна', UID: 'Тестовый Пользователь'}.get(uid, uid)
                        users.append({'participant': {'uid': uid, 'displayname': name}})
                    card['assignedUsers'] = users
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] in ('assignLabel', 'removeLabel') and method == 'PUT':
                    lid = int(self.body()['labelId'])
                    lab = next(l for l in state['board_labels'][board] if l['id'] == lid)
                    card['labels'] = [l for l in card.get('labels', []) if l['id'] != lid] + ([lab] if parts[5] == 'assignLabel' else [])
                    return self.send(200, card)
                if len(parts) == 6 and parts[5] == 'reorder' and method == 'PUT':
                    b = self.body()
                    # Как в Nextcloud: параметры пути важнее тела запроса.
                    card.update(stackId=stack, order=int(b['order']))
                    return self.send(200, [card])
                if len(parts) == 5 and method == 'DELETE':
                    state['cards'].remove(card)
                    return self.send(200, card)

        # --- Файлы (WebDAV) и публикация в беседу
        F = f'/remote.php/dav/files/{UID}/'
        if p.startswith(F):
            path = '/' + unquote(p[len(F):]).strip('/')
            if method == 'MKCOL':
                if path in state['dirs']:
                    return self.send(405)
                state['dirs'].add(path)
                return self.send(201)
            if method == 'PROPFIND':
                return self.propfind(path, F)
            if method == 'GET':
                f = state['files'].get(path)
                if f is None:
                    return self.send(404)
                self.send_response(200); self.send_header('Content-Length', str(len(f['data']))); self.end_headers()
                self.wfile.write(f['data']); return
            if method == 'DELETE':
                if path in state['dirs']:
                    state['dirs'] = {d for d in state['dirs'] if d != path and not d.startswith(path + '/')}
                    for k in [k for k in state['files'] if k.startswith(path + '/')]: del state['files'][k]
                    return self.send(204)
                if state['files'].pop(path, None) is None:
                    return self.send(404)
                return self.send(204)
            if method == 'MOVE':
                dest = '/' + unquote(urlparse(self.headers['Destination']).path[len(F):]).strip('/')
                if dest in state['files'] or dest in state['dirs']:
                    return self.send(412)
                if path in state['files']:
                    state['files'][dest] = state['files'].pop(path)
                elif path in state['dirs']:
                    state['dirs'] = {dest + d[len(path):] if d == path or d.startswith(path + '/') else d for d in state['dirs']}
                    for k in [k for k in state['files'] if k.startswith(path + '/')]: state['files'][dest + k[len(path):]] = state['files'].pop(k)
                else:
                    return self.send(404)
                return self.send(201)
            if method == 'PUT':
                if path.rsplit('/', 1)[0] not in state['dirs'] and path.rsplit('/', 1)[0] != '':
                    return self.send(409)
                n = int(self.headers.get('Content-Length') or 0)
                data = self.rfile.read(n)
                if path in state['files'] and self.headers.get('If-None-Match') == '*':
                    return self.send(412)
                state['files'][path] = {'id': next(ids), 'data': data}
                return self.send(201)
        if method == 'GET' and p == '/ocs/v2.php/cloud/capabilities':
            return self.ocs({'capabilities': {'spreed': {'config': {'attachments': {'allowed': True, 'folder': '/Talk'}}}}})
        SH = '/ocs/v2.php/apps/files_sharing/api/v1/shares'
        if method == 'GET' and p == SH:
            return self.ocs([x for x in state['shares'] if x['path'] == q.get('path')])
        if method == 'DELETE' and p.startswith(SH + '/'):
            sid = p[len(SH) + 1:]
            state['shares'] = [x for x in state['shares'] if x['id'] != sid]
            return self.ocs([])
        if method == 'POST' and p == SH and int(self.peek_json().get('shareType', -1)) in (0, 3):
            b = self.body()
            if b['path'] not in state['files'] and b['path'] not in state['dirs']:
                return self.send(404, {'ocs': {'meta': {'status': 'failure', 'statuscode': 404, 'message': 'Wrong path'}}})
            sid = str(next(ids))
            if int(b['shareType']) == 3:
                sh = {'id': sid, 'share_type': 3, 'path': b['path'], 'share_with': None, 'permissions': 1,
                      'url': f'{base}/s/tok{sid}', 'token': f'tok{sid}', 'expiration': (b.get('expireDate') or '') and b['expireDate'] + ' 00:00:00',
                      'password': 'hash' if b.get('password') else None}
            else:
                u = next((x for x in USERS if x['id'] == b['shareWith']), None)
                if not u: return self.send(404, {})
                sh = {'id': sid, 'share_type': 0, 'path': b['path'], 'share_with': u['id'], 'share_with_displayname': u['label'],
                      'permissions': int(b.get('permissions', 1)), 'url': ''}
            state['shares'].append(sh)
            return self.ocs(sh)
        if method == 'POST' and p == SH:
            b = self.body()
            f = state['files'].get(b.get('path'))
            if int(b.get('shareType', -1)) != 10 or f is None or b.get('shareWith') not in state['messages']:
                return self.send(404, {'message': 'bad share'})
            meta = json.loads(b.get('talkMetaData') or '{}')
            name = b['path'].rsplit('/', 1)[1]
            with lock:
                m = {'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
                     'timestamp': int(time.time()), 'message': meta.get('caption') or '{file}', 'messageType': 'comment', 'systemMessage': '',
                     'messageParameters': {'file': {'type': 'file', 'id': str(f['id']), 'name': name, 'size': len(f['data']), 'mimetype': 'text/plain', 'path': b['path']}}}
                state['messages'][b['shareWith']].append(m)
                lock.notify_all()
            return self.ocs({'id': next(ids)})

        # --- Talk
        if method == 'GET' and p == '/ocs/v2.php/apps/spreed/api/v4/room':
            return self.ocs([room_view(r) for r in state['rooms']])
        if method == 'POST' and p == '/ocs/v2.php/apps/spreed/api/v4/room':
            b = self.body()
            if int(b.get('roomType', 0)) == 1:
                u = next((x for x in USERS if x['id'] == b.get('invite')), None)
                if not u:
                    return self.send(404, {})
                r = next((x for x in state['rooms'] if x['type'] == 1 and x['name'] == u['id']), None)
                if not r:
                    r = {'token': f"dm-{u['id']}", 'type': 1, 'name': u['id'], 'displayName': u['label'], 'readOnly': 0}
                    state['rooms'].append(r); state['messages'][r['token']] = []; state['read'][r['token']] = 0
            else:
                tok = f'grp{next(ids)}'
                r = {'token': tok, 'type': 2, 'name': b['roomName'], 'displayName': b['roomName'], 'readOnly': 0, 'participants': []}
                state['rooms'].append(r); state['messages'][tok] = []; state['read'][tok] = 0
            return self.ocs(room_view(r), 201)
        if method == 'POST' and p.startswith('/ocs/v2.php/apps/spreed/api/v4/room/') and p.endswith('/participants'):
            tok = p.split('/')[-2]
            r = next(x for x in state['rooms'] if x['token'] == tok)
            r.setdefault('participants', []).append(self.body()['newParticipant'])
            return self.ocs({})
        if method == 'GET' and p == '/ocs/v2.php/core/autocomplete/get':
            sq = q.get('search', '').lower()
            return self.ocs([u for u in USERS if sq in (u['label'] + u['id']).lower()])
        R = '/ocs/v2.php/apps/spreed/api/v1/reaction/'
        if p.startswith(R):
            token, mid = p[len(R):].split('/')
            msg = next(m for m in state['messages'][token] if m['id'] == int(mid))
            emoji = self.body().get('reaction') if method == 'POST' else unquote(q.get('reaction', ''))
            reactions = msg.setdefault('reactions', {}); mine = msg.setdefault('reactionsSelf', [])
            if method == 'POST' and emoji not in mine:
                reactions[emoji] = reactions.get(emoji, 0) + 1; mine.append(emoji)
            elif method == 'DELETE' and emoji in mine:
                reactions[emoji] -= 1; mine.remove(emoji)
                if reactions[emoji] <= 0: del reactions[emoji]
            push_update(token, 'reaction' if method == 'POST' else 'reaction_revoked', msg)
            return self.ocs(reactions, 201 if method == 'POST' else 200)
        T = '/ocs/v2.php/apps/spreed/api/v1/chat/'
        if p.startswith(T) and p.endswith('/mentions'):
            sq = q.get('search', '').lower()
            out = [{'id': 'all', 'label': 'УИТ — общий', 'source': 'calls', 'mentionId': 'all'}] if 'all'.startswith(sq) or not sq else []
            out += [{'id': u['id'], 'label': u['label'], 'source': 'users', 'mentionId': u['id']} for u in USERS if sq in u['label'].lower() or sq in u['id']]
            return self.ocs(out)
        if p.startswith(T) and p[len(T):].count('/') == 1 and method in ('PUT', 'DELETE'):
            token, mid = p[len(T):].split('/')
            msg = next(m for m in state['messages'][token] if m['id'] == int(mid))
            if msg['actorId'] != UID:
                return self.send(403, {})
            if method == 'PUT':
                b = self.body()
                text, params = b['message'], {}
                import re as _re
                for i, mm in enumerate(_re.findall(r'@"([^"]+)"', text)):
                    u = next((x for x in USERS if x['id'] == mm), None)
                    if u:
                        params[f'mention-user{i}'] = {'type': 'user', 'id': u['id'], 'name': u['label'], 'mention-id': u['id']}
                        text = text.replace(f'@"{mm}"', '{' + f'mention-user{i}' + '}', 1)
                msg.update(message=text, messageParameters=params or [], lastEditTime=int(time.time()))
                push_update(token, 'message_edited', msg)
            else:
                msg.update(message='Message deleted by you', messageType='comment_deleted', messageParameters=[], reactions={}, reactionsSelf=[])
                push_update(token, 'message_deleted', msg)
            return self.ocs(msg)
        if p.startswith(T):
            token = p[len(T):]
            if token not in state['messages']:
                return self.send(404)
            if method == 'POST':
                b = self.body()
                with lock:
                    import re as _re
                    text, params = b['message'], {}
                    for i, mm in enumerate(_re.findall(r'@"([^"]+)"', text)):
                        u = next((x for x in USERS if x['id'] == mm), None)
                        if u:
                            params[f'mention-user{i}'] = {'type': 'user', 'id': u['id'], 'name': u['label'], 'mention-id': u['id']}
                            text = text.replace(f'@"{mm}"', '{' + f'mention-user{i}' + '}', 1)
                    m = {'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
                         'timestamp': int(time.time()), 'message': text, 'messageType': 'comment', 'systemMessage': '',
                         'messageParameters': params or [], 'raw': b['message']}
                    if b.get('replyTo'):
                        parent = next(x for x in state['messages'][token] if x['id'] == int(b['replyTo']))
                        m['parent'] = parent
                    state['messages'][token].append(m)
                    lock.notify_all()
                return self.ocs(m, 201)
            future = q.get('lookIntoFuture') == '1'
            last = int(q.get('lastKnownMessageId', 0))
            limit = int(q.get('limit', 100))
            with lock:
                if future:
                    deadline = time.time() + min(int(q.get('timeout', 30)), 60)
                    while not [m for m in state['messages'][token] if m['id'] > last] and time.time() < deadline:
                        lock.wait(deadline - time.time())
                    msgs = [m for m in state['messages'][token] if m['id'] > last][:limit]
                else:
                    older = [m for m in state['messages'][token] if not last or m['id'] < last]
                    msgs = list(reversed(older[-limit:]))  # история приходит от новых к старым
                if msgs and q.get('setReadMarker', '1') == '1':
                    state['read'][token] = max(state['read'][token], max(m['id'] for m in msgs))
            if not msgs:
                self.send_response(304); self.send_header('Content-Length', '0'); self.end_headers()
                return
            return self.ocs(msgs, 200, {'X-Chat-Last-Given': str(msgs[-1]['id'])})

        self.send(404, {'message': f'no route {method} {p}'})

    def do_GET(self): self.route('GET')
    def do_POST(self): self.route('POST')
    def do_PUT(self): self.route('PUT')
    def do_DELETE(self): self.route('DELETE')
    def do_MKCOL(self): self.route('MKCOL')
    def do_PROPFIND(self): self.route('PROPFIND')
    def do_MOVE(self): self.route('MOVE')

    def propfind(self, path, prefix):
        from urllib.parse import quote
        n = int(self.headers.get('Content-Length') or 0)
        if n: self.rfile.read(n)
        if path not in state['dirs'] and path not in state['files']:
            return self.send(404)
        def entry(pth, is_dir):
            href = prefix.rstrip('/') + quote(pth) + ('/' if is_dir and pth != '/' else '')
            fid = state['dir_ids'].setdefault(pth, next(ids)) if is_dir else state['files'][pth]['id']
            shared = any(x['path'] == pth for x in state['shares'])
            props = f'<oc:fileid>{fid}</oc:fileid><oc:permissions>RGDNVCK</oc:permissions><d:getlastmodified>Tue, 27 Oct 2026 10:00:00 GMT</d:getlastmodified>'
            props += '<oc:share-types>' + ('<oc:share-type>3</oc:share-type>' if shared else '') + '</oc:share-types>'
            if is_dir:
                size = sum(len(v['data']) for k, v in state['files'].items() if k.startswith(pth.rstrip('/') + '/'))
                props += f'<d:resourcetype><d:collection/></d:resourcetype><oc:size>{size}</oc:size>'
            else:
                props += f'<d:resourcetype/><d:getcontentlength>{len(state["files"][pth]["data"])}</d:getcontentlength><d:getcontenttype>text/plain</d:getcontenttype>'
            return f'<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>'
        items = [entry(path, path in state['dirs'])]
        if path in state['dirs'] and self.headers.get('Depth') == '1':
            base_ = path.rstrip('/') + '/'
            children = {d for d in state['dirs'] if d.startswith(base_) and '/' not in d[len(base_):] and d != path}
            items += [entry(d, True) for d in sorted(children)]
            items += [entry(f, False) for f in sorted(state['files']) if f.startswith(base_) and '/' not in f[len(base_):]]
        body = ('<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns">' + ''.join(items) + '</d:multistatus>').encode()
        self.send_response(207); self.send_header('Content-Type', 'application/xml; charset=utf-8')
        self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)


cert, key = sys.argv[1:3]
port = int(sys.argv[3]) if len(sys.argv) > 3 else 8443
srv = ThreadingHTTPServer(('127.0.0.1', port), H)
ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
ctx.load_cert_chain(cert, key)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print(f'Nextcloud mock on https://localhost:{port}', flush=True)
srv.serve_forever()
