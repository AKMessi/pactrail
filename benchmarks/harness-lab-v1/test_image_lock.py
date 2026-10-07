import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("image_lock", Path(__file__).with_name("image_lock.py"))
image_lock = importlib.util.module_from_spec(spec)
spec.loader.exec_module(image_lock)


class ImageIdentityTests(unittest.TestCase):
    def test_index_to_platform_payload_is_digest_bound_without_retaining_token(self):
        manifest = json.dumps({"layers": [{"size": 42, "digest": "sha256:" + "a" * 64}]}).encode()
        digest = image_lock.digest(manifest)
        index = json.dumps({"manifests": [{"platform": {"os": "linux", "architecture": "amd64"},
                                           "digest": digest}]}).encode()
        def transport(url, headers=None):
            if "auth.docker.io" in url:
                return b'{"token":"fixture-only-token"}', {}
            self.assertEqual(headers["Authorization"], "Bearer fixture-only-token")
            return (manifest, {}) if url.endswith(digest) else (index, {"Docker-Content-Digest": image_lock.digest(index)})
        row = image_lock.resolve("swebench/fixture:latest", transport)
        self.assertEqual(row["immutable_image"], "swebench/fixture@" + digest)
        self.assertEqual(row["compressed_layer_bytes"], 42)
        self.assertNotIn("fixture-only-token", json.dumps(row))

    def test_wrong_digest_and_ambiguous_platform_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            image_lock.resolve("swebench/fixture:latest", lambda url, headers=None:
                               (b'{"token":"fixture"}', {}) if "auth.docker.io" in url
                               else (b'{}', {"Docker-Content-Digest": "sha256:" + "b" * 64}))
        entry = {"platform": {"os": "linux", "architecture": "amd64"}, "digest": "sha256:" + "c" * 64}
        for values in ([], [entry, entry]):
            with self.assertRaises(ValueError):
                image_lock.compatible_manifest({"manifests": values})

    def test_namespace_and_malformed_platform_are_rejected_without_network(self):
        def forbidden(*_args):
            self.fail("rejected image reached network")
        for image in ("evil/fixture:latest", "https://evil/fixture", "swebench/../fixture:latest"):
            with self.assertRaises(ValueError):
                image_lock.resolve(image, forbidden)
        for payload in ({"manifests": None}, {"manifests": [None]}, {"manifests": [{"platform": []}]}):
            with self.assertRaises(ValueError):
                image_lock.compatible_manifest(payload)


if __name__ == "__main__":
    unittest.main()
