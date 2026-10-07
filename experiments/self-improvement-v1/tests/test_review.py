from pathlib import Path
import sys
import unittest
import http.client
import json
import threading

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.review import authorize, create_server
from pactrail_lab.safe import Refusal
import test_campaign


class ReviewHttp(unittest.TestCase):
    def test_actual_cookie_origin_approval_idempotence_and_undo(self):
        fixture = test_campaign.CampaignTests()
        fixture.setUp()
        server = None
        try:
            candidate = fixture.candidate()
            verdict = fixture.campaign.evaluate("evaluate", fixture.store.head(), candidate)
            server = create_server(fixture.store.root, 0)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            host = "127.0.0.1:" + str(server.server_port)
            cookie = None
            def request(method, path, body=None, origin=None, hostname=host):
                connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=5)
                headers = {"Host": hostname, "Content-Type": "application/json"}
                if cookie: headers["Cookie"] = cookie
                if origin: headers["Origin"] = origin
                connection.request(method, path, json.dumps(body) if body is not None else None, headers)
                response = connection.getresponse()
                result = response.status, dict(response.getheaders()), response.read()
                connection.close()
                return result
            code, headers, _ = request("GET", "/")
            self.assertEqual(code, 200)
            self.assertIn("default-src 'none'", headers["Content-Security-Policy"])
            self.assertIn("HttpOnly; SameSite=Strict", headers["Set-Cookie"])
            self.assertEqual(request("GET", "/api/status")[0], 403)
            cookie = headers["Set-Cookie"].split(";")[0]
            self.assertEqual(request("GET", "/api/status", hostname="attacker.example")[0], 403)
            payload = dict(command_id="http-approve", head=fixture.store.head(),
                           verdict=verdict["verdict"], acknowledgment=True)
            for origin in (None, "https://attacker.example"):
                self.assertEqual(request("POST", "/api/approve", payload, origin)[0], 409)
                self.assertEqual(fixture.store.value("active"), fixture.parent)
            self.assertEqual(request("POST", "/api/approve", {**payload, "head": "0" * 64}, "http://" + host)[0], 409)
            result = request("POST", "/api/approve", payload, "http://" + host)
            self.assertEqual(result[0], 200)
            self.assertEqual(json.loads(result[2])["active"], candidate)
            self.assertEqual(request("POST", "/api/approve", payload, "http://" + host)[0], 200)
            undo = dict(command_id="http-undo", head=fixture.store.head(), revision=candidate)
            self.assertEqual(request("POST", "/api/undo", undo, "http://" + host)[0], 200)
            self.assertEqual(fixture.store.value("active"), fixture.parent)
            self.assertFalse(fixture.store.value("qualified"))
            fixture.store.verify()
        finally:
            if server:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
            fixture.tearDown()


class ReviewSecurity(unittest.TestCase):
    def test_exact_origin_host_and_session(self):
        host, token = "127.0.0.1:3091", "test-session"
        authorize(host, "http://" + host, "pactrail_lab=" + token, host, token, mutation=True)
        for headers in (("attacker.example:3091", "http://" + host, "pactrail_lab=" + token),
                        (host, "https://attacker.example", "pactrail_lab=" + token),
                        (host, None, "pactrail_lab=" + token),
                        (host, "http://" + host, "pactrail_lab=wrong")):
            with self.assertRaises(Refusal): authorize(*headers, host, token, mutation=True)

    def test_ui_has_no_inline_or_html_injection_escape(self):
        web = Path(__file__).resolve().parents[1] / "web"
        html = (web / "index.html").read_text()
        script = (web / "app.js").read_text()
        self.assertNotIn("style=", html)
        self.assertNotIn("innerHTML", script)
        self.assertNotIn("eval(", script)
        self.assertIn('id="cancel" value="cancel" autofocus', html)


if __name__ == "__main__": unittest.main()
