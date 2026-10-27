"""Имитация Nextcloud (Login Flow v2, Deck API v1.0, Talk API v4/v1) для тестов.
Форматы ответов — по официальной документации Deck и Talk.
python3 tests/servers/nextcloud_mock.py <cert> <key> [port=8443]

Тестовые ручки:
  POST /test/inject?token=<room>&text=...  — сообщение «от коллеги» (будит long-poll)
  GET  /test/state                         — состояние для проверок
"""
import json, ssl, sys, threading, time, base64, itertools
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

LOGIN, APP_PASSWORD, UID = 'test@rrcki.ru', 'app-pass-123', 'test'
lock = threading.Condition()
ids = itertools.count(100)
state = {
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
        {'id': 1, 'stackId': 10, 'title': 'Обновить RuPost', 'description': 'Окно работ в среду', 'type': 'plain', 'order': 0,
         'duedate': '2026-10-30T00:00:00+00:00', 'done': None, 'archived': False, 'owner': {'uid': UID, 'displayname': 'Тестовый'},
         'labels': [{'id': 1, 'title': 'Срочно', 'color': 'e9322d', 'boardId': 1}],
         'assignedUsers': [{'participant': {'uid': 'ivanov', 'displayname': 'Иванов Иван'}}], 'commentsUnread': 1, 'attachmentCount': 0},
        {'id': 2, 'stackId': 11, 'title': 'Настроить брокер X2Go', 'description': '', 'type': 'plain', 'order': 0,
         'duedate': None, 'done': None, 'archived': False, 'owner': UID, 'labels': [], 'assignedUsers': [], 'commentsUnread': 0},
        {'id': 3, 'stackId': 20, 'title': 'Плановые работы 1 ноября', 'description': '', 'type': 'plain', 'order': 0,
         'duedate': None, 'done': None, 'archived': False, 'owner': 'admin', 'labels': [], 'assignedUsers': [], 'commentsUnread': 0},
    ],
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


def room_view(r):
    msgs = [m for m in state['messages'][r['token']] if m['systemMessage'] != 'reaction']
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

    def body(self):
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

        # --- тестовые ручки
        if p == '/test/state':
            return self.send(200, {k: state[k] for k in ('polls', 'revoked', 'cards', 'stacks')} | {'messages': state['messages']})
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
        if method == 'GET' and p == '/ocs/v1.php/cloud/user':
            return self.ocs({'id': UID, 'displayname': 'Тестовый Пользователь', 'email': LOGIN})
        if method == 'DELETE' and p == '/ocs/v2.php/core/apppassword':
            state['revoked'] = True
            return self.ocs([])

        # --- Deck
        D = '/index.php/apps/deck/api/v1.0/boards'
        if p == D and method == 'GET':
            return self.send(200, state['boards'])
        parts = p[len(D):].strip('/').split('/') if p.startswith(D + '/') else []
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
                if len(parts) == 6 and parts[5] == 'reorder' and method == 'PUT':
                    b = self.body()
                    card.update(stackId=int(b['stackId']), order=int(b['order']))
                    return self.send(200, [card])
                if len(parts) == 5 and method == 'DELETE':
                    state['cards'].remove(card)
                    return self.send(200, card)

        # --- Talk
        if method == 'GET' and p == '/ocs/v2.php/apps/spreed/api/v4/room':
            return self.ocs([room_view(r) for r in state['rooms']])
        T = '/ocs/v2.php/apps/spreed/api/v1/chat/'
        if p.startswith(T):
            token = p[len(T):]
            if token not in state['messages']:
                return self.send(404)
            if method == 'POST':
                b = self.body()
                with lock:
                    m = {'id': next(ids), 'actorType': 'users', 'actorId': UID, 'actorDisplayName': 'Тестовый Пользователь',
                         'timestamp': int(time.time()), 'message': b['message'], 'messageType': 'comment', 'systemMessage': '',
                         'messageParameters': []}
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


cert, key = sys.argv[1:3]
port = int(sys.argv[3]) if len(sys.argv) > 3 else 8443
srv = ThreadingHTTPServer(('127.0.0.1', port), H)
ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
ctx.load_cert_chain(cert, key)
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
print(f'Nextcloud mock on https://localhost:{port}', flush=True)
srv.serve_forever()
