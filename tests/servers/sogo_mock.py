"""Мини-мок веб-API SOGo: POST /SOGo/connect (JSON) → cookie + XSRF,
GET /SOGo/so/<user>/Contacts/allContactSearch?search=… → {"contacts": [...]}."""
import json, ssl, sys
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs, unquote

USER, PASS = 'test@rrcki.ru', 'Secret-123'
PEOPLE = [
    {'c_name': 'ivanov', 'c_cn': 'Иванов Иван', 'c_mail': 'ivanov@nrcki.ru', 'c_o': 'НИЦ КИ', 'sourceid': 'gal'},
    {'c_name': 'sidorov', 'c_cn': 'Сидоров Олег', 'c_mail': ['sidorov@nrcki.ru'], 'c_title': 'Аналитик', 'sourceid': 'gal'},
]

class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def send(self, code, obj=None, headers=()):
        body = b'' if obj is None else json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(body)))
        for k, v in headers: self.send_header(k, v)
        self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        n = int(self.headers.get('Content-Length') or 0); b = json.loads(self.rfile.read(n) or b'{}')
        if self.path == '/SOGo/connect' and b.get('userName') == USER and b.get('password') == PASS:
            return self.send(200, {'expire': -1}, [('Set-Cookie', '0xHIGHFLYxSOGo=basic%20abc; path=/SOGo/; secure; HttpOnly'), ('Set-Cookie', 'XSRF-TOKEN=tok123; path=/SOGo/')])
        self.send(403, {})
    def do_GET(self):
        u = urlparse(self.path); q = {k: v[0] for k, v in parse_qs(u.query).items()}
        ok = '0xHIGHFLYxSOGo=basic%20abc' in (self.headers.get('Cookie') or '') and self.headers.get('X-XSRF-TOKEN') == 'tok123'
        if unquote(u.path) == f'/SOGo/so/{USER}/Contacts/allContactSearch':
            if not ok: return self.send(302, None, [('Location', '/SOGo/')])
            s = q.get('search', '').lower()
            return self.send(200, {'contacts': [p for p in PEOPLE if s in json.dumps(p, ensure_ascii=False).lower()]})
        if unquote(u.path) == f'/SOGo/so/{USER}/logoff':
            return self.send(200, {})
        self.send(404, {})

srv = ThreadingHTTPServer(('127.0.0.1', int(sys.argv[3])), H)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(sys.argv[1], sys.argv[2])
srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
srv.serve_forever()
