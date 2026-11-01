#!/usr/bin/env python3
"""Заглушка сервиса orders для ручной приёмки L2: POST /v1/orders -> 201 {id}, GET /v1/orders/<id> -> 200.

Запуск: python3 dev/stub/orders_stub.py [порт=8080]. Только стандартная библиотека Python 3;
HTTP/1.1 с keep-alive, поток на соединение. POST без заголовка Authorization -> 401: так видно,
что секрет стенда дошёл до запроса. Kafka заглушка не изображает.
"""
import json
import sys
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _json(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _consume_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def do_POST(self):
        raw = self._consume_body()
        if self.path.split("?")[0] != "/v1/orders":
            return self._json(404, {"error": "not found"})
        if self.headers.get("Authorization", "") == "":
            return self._json(401, {"error": "no token"})
        try:
            body = json.loads(raw or b"{}")
        except ValueError:
            return self._json(400, {"error": "bad json"})
        self._json(
            201,
            {
                "id": str(uuid.uuid4()),
                "status": "created",
                "customerId": body.get("customerId"),
                "items": body.get("items", []),
            },
        )

    def do_GET(self):
        parts = self.path.split("?")[0].split("/")
        if len(parts) == 4 and parts[1] == "v1" and parts[2] == "orders" and parts[3]:
            return self._json(200, {"id": parts[3], "status": "created", "items": []})
        self._json(404, {"error": "not found"})

    def log_message(self, *_args):  # тихо: сотни запросов в секунду
        pass


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    server.daemon_threads = True
    print(f"orders stub on http://127.0.0.1:{port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
