"""Exercise generated content redirects against Caddy, including credential exclusions."""
import http.client
import importlib.machinery
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
module = importlib.machinery.SourceFileLoader("sites_redirects", str(ROOT / "infra/scripts/sites-redirects")).load_module()


class Redirects(unittest.TestCase):
    def test_reject_ambiguous_or_unsafe_mapping(self):
        for source, target in [("api.finite.chat", "hello.finite.site"), ("x.finite.chat", "evil.example"), ("x.finite.chat/foo", "hello.finite.site"), ("x.y.finite.chat", "hello.finite.site"), ("hello.v2.finite.chat", "hello.finite.site"), ("x.finite.chat", "hello.finite.site/path")]:
            with self.assertRaises(ValueError):
                module.render([dict(from_host=source, to_host=target)])
        row = dict(from_host="old.finite.chat", to_host="new.finite.site")
        with self.assertRaises(ValueError):
            module.render([row, row])

    def test_real_caddy_redirects(self):
        # Read the actual host matchers and handlers. Only TLS/listening and the
        # private mapping path change for this isolated, loopback-only fixture.
        hosts = json.loads(subprocess.check_output([
            "nix", "eval", "--json",
            ".#nixosConfigurations.finite-lat-2.config.services.caddy.virtualHosts",
            "--apply", "hosts: builtins.mapAttrs (_: host: host.extraConfig) hosts",
        ], cwd=ROOT, text=True))
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                port = listener.getsockname()[1]
            rows = [dict(from_host="old.finite.chat", to_host="new.finite.site"), dict(from_host="guide.docs.finite.chat", to_host="guide.finite.site")]
            config = root / "Caddyfile"
            mapping = root / "sites-redirects.caddy"
            mapping.write_text(module.render(rows))
            blocks = []
            for host, handler in hosts.items():
                if not host.endswith(".finite.chat"):
                    continue
                handler = "\n".join(line for line in handler.splitlines()
                                    if not line.strip().startswith("tls "))
                handler = handler.replace("/etc/finite/sites-redirects.caddy", str(mapping))
                blocks.append(f"http://{host}:{port} {{\n bind 127.0.0.1\n{handler}\n}}")
            self.assertTrue(blocks, "no production Sites host handlers were evaluated")
            config.write_text("{\n admin off\n auto_https off\n}\n" + "\n".join(blocks))

            with (root / "caddy.log").open("w") as log:
                process = subprocess.Popen(["caddy", "run", "--config", str(config), "--adapter", "caddyfile"], stdout=log, stderr=log, env={**os.environ, "XDG_CONFIG_HOME": scratch, "XDG_DATA_HOME": scratch})
                def request(host, path, method="GET"):
                    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
                    conn.request(method, path, headers={"Host": host})
                    response = conn.getresponse()
                    result = response.status, dict(response.getheaders())
                    response.read()
                    conn.close()
                    return result
                try:
                    for _ in range(100):
                        if process.poll() is not None:
                            self.fail((root / "caddy.log").read_text())
                        try:
                            request("unknown.finite.chat", "/")
                            break
                        except OSError:
                            time.sleep(0.05)
                    for row in rows:
                        for path in ["/", "/nested?x=1&x=2", "/a%20b?x=%2F&y=a+b"]:
                            for method in ["GET", "HEAD"]:
                                status, headers = request(row["from_host"], path, method)
                                self.assertEqual(status, 302)
                                self.assertEqual(headers["Location"], "https://" + row["to_host"] + path)
                                self.assertEqual(headers["Cache-Control"], "no-store")
                                self.assertEqual(headers["Referrer-Policy"], "no-referrer")
                        for path in ["/_finite", "/_finite/auth?token=synthetic", "/%5ffinite/auth?token=synthetic", "/_finite%2fauth?token=synthetic"]:
                            status, headers = request(row["from_host"], path)
                            self.assertEqual(status, 410)
                            self.assertNotIn("Location", headers)
                        for method in ["POST", "PUT", "PATCH", "DELETE"]:
                            status, headers = request(row["from_host"], "/", method)
                            self.assertEqual(status, 410)
                            self.assertNotIn("Location", headers)
                    for host in ["unknown.finite.chat", "unknown.docs.finite.chat", "api.finite.chat", "git.finite.chat"]:
                        for method in ["GET", "HEAD", "POST"]:
                            status, headers = request(host, "/", method)
                            self.assertEqual(status, 410)
                            self.assertNotIn("Location", headers)
                    # Extra-label validation URLs are outside the edge contract,
                    # and the generator must refuse to promise a redirect for them.
                    for host in ["hello.v2.finite.chat", "a.old.finite.chat"]:
                        with self.assertRaises(ValueError):
                            module.render([dict(from_host=host, to_host="new.finite.site")])
                        self.assertNotIn("Location", request(host, "/")[1])
                finally:
                    process.terminate()
                    process.wait(timeout=10)


if __name__ == "__main__":
    unittest.main()
