"""Servidor mínimo de recorte: POST / (corpo = imagem) devolve PNG com fundo transparente. GET / = saúde.
Um pedido por vez (o modelo usa bastante memória)."""
import os
from http.server import BaseHTTPRequestHandler, HTTPServer

from rembg import new_session, remove

SESSAO = new_session(os.environ.get("MODELO", "isnet-general-use"))
MAXIMO = 25 * 1024 * 1024


class Recorte(BaseHTTPRequestHandler):
    def responder(self, codigo, corpo, tipo="text/plain"):
        self.send_response(codigo)
        self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(corpo)))
        self.end_headers()
        self.wfile.write(corpo)

    def do_GET(self):
        self.responder(200, b"ok")

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0 or n > MAXIMO:
            return self.responder(400, b"imagem vazia ou grande demais")
        try:
            png = remove(self.rfile.read(n), session=SESSAO, post_process_mask=True)
        except Exception as e:  # imagem inválida etc.
            return self.responder(422, str(e).encode())
        self.responder(200, png, "image/png")

    def log_message(self, *args):
        pass


HTTPServer(("0.0.0.0", 7000), Recorte).serve_forever()
