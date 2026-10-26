"""Тестовый SMTPS-сервер (порт 465, TLS сразу, AUTH PLAIN/LOGIN).
Принятые письма сохраняются в каталог OUT как .eml.
python3 tests/servers/smtp_server.py <cert> <key> <out-dir> [port]"""
import asyncio, ssl, sys, time, os
from aiosmtpd.controller import Controller
from aiosmtpd.smtp import AuthResult, LoginPassword

USERS = {'test@rrcki.ru': 'Secret-123'}

class Auth:
    def __call__(self, server, session, envelope, mechanism, auth_data):
        if isinstance(auth_data, LoginPassword):
            ok = USERS.get(auth_data.login.decode()) == auth_data.password.decode()
            return AuthResult(success=ok, handled=False)
        return AuthResult(success=False, handled=False)

class Store:
    def __init__(self, out):
        self.out = out
    async def handle_DATA(self, server, session, envelope):
        name = os.path.join(self.out, f'{time.time():.6f}.eml')
        with open(name, 'wb') as f:
            f.write(f'X-Envelope-From: {envelope.mail_from}\r\nX-Envelope-To: {",".join(envelope.rcpt_tos)}\r\n'.encode())
            f.write(envelope.original_content)
        return '250 OK'

cert, key, out = sys.argv[1:4]
port = int(sys.argv[4]) if len(sys.argv) > 4 else 465
os.makedirs(out, exist_ok=True)
ctx = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
ctx.load_cert_chain(cert, key)
c = Controller(Store(out), hostname='127.0.0.1', port=port, ssl_context=ctx,
               authenticator=Auth(), auth_required=True, auth_require_tls=False)
c.start()
print(f'SMTPS on 127.0.0.1:{port}', flush=True)
try:
    while True:
        time.sleep(3600)
except KeyboardInterrupt:
    c.stop()
