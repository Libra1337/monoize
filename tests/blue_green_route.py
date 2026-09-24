"""Run with unshare --net python3 tests/blue_green_route.py in a Linux checkout."""

from pathlib import Path
import socket
import os
import socketserver
import subprocess
import threading
import time


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        while self.request.recv(1):
            self.request.sendall(self.server.identity)


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True


def main():
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    helper = Path(__file__).resolve().parents[1] / "scripts/blue-green-route.sh"
    servers = []
    clients = []
    for port, identity in [(8081, b"old"), (8080, b"new")]:
        server = Server(("127.0.0.1", port), Handler)
        server.identity = identity
        threading.Thread(target=server.serve_forever, daemon=True).start()
        servers.append(server)

    def connect(source="127.0.0.1", proxy=True):
        os.seteuid(19991 if proxy else 0)
        try:
            client = socket.create_connection(
                ("127.0.0.1", 8081), timeout=3, source_address=(source, 0)
            )
        finally:
            os.seteuid(0)
        clients.append(client)
        return client

    def expect(client, identity):
        client.sendall(b"?")
        received = b""
        while len(received) < len(identity):
            chunk = client.recv(len(identity) - len(received))
            assert chunk, "connection closed during route update"
            received += chunk
        assert received == identity

    def route(active):
        subprocess.run(["bash", str(helper), "8081", str(active), "19991"], check=True)

    try:
        old = connect()
        expect(old, b"old")
        route(8080)
        new = connect()
        expect(new, b"new")
        expect(old, b"old")
        bypass = connect("127.0.0.2")
        expect(bypass, b"old")
        direct = connect(proxy=False)
        expect(direct, b"old")

        for active in [8081, 8080] * 25:
            route(active)
            current = connect()
            expect(current, b"old" if active == 8081 else b"new")
            current.close()
            expect(old, b"old")
            expect(new, b"new")

        old.close()
        bypass.close()
        direct.close()
        for _ in range(50):
            sockets = subprocess.check_output(
                ["ss", "-tnH", "state", "established", "( sport = :8081 )"]
            )
            if not sockets.strip():
                break
            time.sleep(0.02)
        assert not sockets.strip(), "redirected client sockets must not prevent old-server drain"
        expect(new, b"new")
        subprocess.run(["bash", str(helper), "--check", "8081", "8080", "19991"], check=True)
        print("PASS: existing flows, new flows, reverse swaps, direct probes, and drain counts")
    finally:
        for client in clients:
            client.close()
        for server in servers:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    main()
