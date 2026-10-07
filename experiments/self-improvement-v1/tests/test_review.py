from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from pactrail_lab.review import authorize
from pactrail_lab.safe import Refusal


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
