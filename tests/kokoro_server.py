"""Local TLS fixture for the public Kokoro endpoint. No external traffic required."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import json
from urllib.parse import parse_qs
import ssl
import sys

work = Path(sys.argv[1])
tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
tls.load_cert_chain(work / "cert", work / "key")


OPTIONS = {
    "formats": [{"value": "mihomo"}],
    "plans": [{"name": "Basic", "supported_isps": ["all"]}, {"name": "Premium", "supported_isps": ["ct"]}],
    "protocols": [{"value": "vmess", "label": "VMess", "supports_direct": False},
                  {"value": "anytls", "label": "AnyTLS", "supports_direct": True}],
    "isps": [{"value": "", "label": "Default"}, {"value": "ct", "label": "Telecom"}, {"value": "cu", "label": "Unicom"}],
    "rule_sources": ["origin", "mirror"], "final_routes": ["proxy", "direct"],
    "profile_update": {"min_hours": 1, "max_hours": 72},
    "defaults": {"protocol": "vmess", "mode": "relay", "rule_source": "origin", "final_route": "proxy",
                 "rule_provider_auto_update": True, "profile_auto_update": True, "profile_update_hours": 1},
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_CONNECT(self):
        if self.path != "amamiyakoko.ro:443":
            self.send_error(403)
            return
        self.send_response(200)
        self.end_headers()
        with tls.wrap_socket(self.connection, server_side=True) as sock:
            Handler(sock, self.client_address, self.server)

    def reply(self, status, data):
        self.send_response(status)
        body = json.dumps(data).encode()
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)

    def authorized(self):
        return self.server.active and self.headers.get("Authorization") == f"Bearer fixture-access-{self.server.generation}"

    def tokens(self):
        self.server.generation += 1
        self.server.active = True
        n = self.server.generation
        self.reply(200, {"token_type": "Bearer", "access_token": f"fixture-access-{n}",
                         "refresh_token": f"fixture-refresh-{n}", "expires_in": 1800, "refresh_expires_in": 86400})

    def do_POST(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        if self.headers.get_content_type() == "application/x-www-form-urlencoded":
            form = parse_qs(body.decode())
            assert form["client_id"] == ["kokorobox-shell"]
            if self.path == "/api/app/auth/device":
                if (work / "device-unavailable").exists():
                    self.reply(404, {})
                    return
                ttl = 1 if (work / "device-expiring").exists() else 60
                self.reply(200, {"device_code": "fixture-device-secret", "user_code": "ABCD-EFGH",
                                 "verification_uri": "https://amamiyakoko.ro/device", "interval": 1, "expires_in": ttl})
                return
            assert self.path == "/api/app/auth/token"
            assert form["grant_type"] == ["urn:ietf:params:oauth:grant-type:device_code"]
            assert not (work / "device-expiring").exists(), "Poll after expiration"
            assert form["device_code"] == ["fixture-device-secret"]
            if self.server.polls == 0:
                self.reply(400, {"error": "authorization_pending"})
            else:
                self.tokens()
            self.server.polls += 1
            return
        assert self.headers.get_content_type() == "application/json"
        data = json.loads(body)
        if self.path == "/api/app/auth/token":
            if not self.server.active or data != {"grant_type": "refresh_token", "refresh_token": f"fixture-refresh-{self.server.generation}"}:
                self.reply(401, {})
                return
            self.tokens()
        elif not self.authorized():
            self.reply(401, {})
        elif self.path == "/api/app/auth/revoke":
            self.server.active = False
            (work / "revoked").touch()
            self.send_response(204)
            self.end_headers()
        elif self.path == "/api/app/subscription/resolve":
            assert data.get("format") == "mihomo"
            (work / "resolved.json").write_text(json.dumps(data))
            if (work / "reject-selection").exists():
                self.reply(422, {})
                return
            url = "https://amamiyakoko.ro/api/app/subscription/config?format=mihomo"
            if (work / "bad-origin").exists():
                url = url.replace("amamiyakoko.ro", "example.invalid")
            self.reply(200, {"format": "mihomo", "content_type": "text/yaml", "authenticated_config_url": url})
        else:
            self.send_error(404)

    def do_GET(self):
        if self.path == "/provider":
            rules = b"payload:\n  - DOMAIN,provider.test\n"
            self.send_response(200)
            self.send_header("Content-Length", str(len(rules)))
            self.end_headers()
            self.wfile.write(rules)
            return
        if self.path == "/probe":
            self.send_response(200)
            self.send_header("Content-Length", "3")
            self.end_headers()
            self.wfile.write(b"ok\n")
            return
        account_profile = self.path == "/api/app/subscription/config?format=mihomo"
        if self.path.startswith("/api/app/"):
            if not self.authorized() or (self.server.generation == 1 and (work / "require-refresh").exists()):
                self.reply(401, {})
                return
            if self.path == "/api/app/subscription/options":
                self.reply(200, OPTIONS)
                return
            if self.path == "/api/app/me":
                self.reply(200, {"osu_id": "456" if (work / "other-account").exists() else "123", "username": "Fixture User", "proxy_uuid": "fixture-private-uuid"})
                return
            assert account_profile
        else:
            assert self.path == "/api/config?uuid=synthetic-fixture&client=meta"
            assert not self.headers.get("Authorization")
        assert self.headers["User-Agent"].startswith("KokoroBox-Shell/")
        mode = (work / "mode").read_text().strip() if (work / "mode").exists() else "initial"
        if mode in ("redirect", "denied"):
            self.send_response(302 if mode == "redirect" else 403)
            self.send_header("Content-Length", "0")
            self.send_header("Location", "/probe")
            self.end_headers()
            return
        rules = "- DOMAIN,one.test,DIRECT\n" if mode == "updated" else ""
        if account_profile:
            rules += "- DOMAIN,account.test,DIRECT\n"
        data = ("proxies:\n- name: fixture\n  type: http\n  server: 127.0.0.1\n  port: 48000\n"
                "- name: fixture-vmess\n  type: vmess\n  server: 127.0.0.1\n  port: 48001\n"
                "  uuid: 00000000-0000-4000-8000-000000000001\n  alterId: 0\n  cipher: auto\n"
                "- name: fixture-ss\n  type: ss\n  server: 127.0.0.1\n  port: 48002\n"
                "  cipher: aes-128-gcm\n  password: fixture-secret\n"
                "proxy-groups:\n- name: Group\n  type: select\n  proxies: [fixture]\n"
                "rule-providers:\n  kokoro-test:\n    type: http\n    behavior: classical\n"
                f"    url: http://127.0.0.1:{self.server.server_port}/provider\n"
                "    path: ./ruleset/kokoro-test.yaml\n    interval: 3600\n"
                "rules:\n- DOMAIN,user.test,REJECT\n- RULE-SET,kokoro-test,DIRECT\n" + rules + "- MATCH,DIRECT\n")
        body = b"proxies: [broken" if mode == "invalid" else data.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/yaml")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
    server.polls = server.generation = 0
    server.active = False
    (work / "port").write_text(str(server.server_port))
    server.serve_forever()
