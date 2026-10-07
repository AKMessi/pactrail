#!/usr/bin/env python3
"""Resolve declared public SWE-bench images to immutable Linux/amd64 digests.

Fetches bounded manifests only, never image layers. Anonymous pull tokens remain
in memory; no user/provider credential or daemon is required. This is preparation,
not proof that an image executes or that a grader passes.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import urllib.parse
import urllib.request

MAX_RESPONSE = 1024 * 1024
ACCEPT = ", ".join(("application/vnd.oci.image.index.v1+json",
                    "application/vnd.docker.distribution.manifest.list.v2+json",
                    "application/vnd.oci.image.manifest.v1+json",
                    "application/vnd.docker.distribution.manifest.v2+json"))


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


def fetch(url, headers=None):
    request = urllib.request.Request(url, headers={"User-Agent": "Pactrail-Harness-Lab/1", **(headers or {})})
    with urllib.request.build_opener(NoRedirect).open(request, timeout=30) as response:
        raw = response.read(MAX_RESPONSE + 1)
        if len(raw) > MAX_RESPONSE:
            raise ValueError("registry response exceeds bound")
        return raw, dict(response.headers)


def digest(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def compatible_manifest(index):
    if not isinstance(index, dict) or not isinstance(index.get("manifests"), list):
        raise ValueError("manifest index must contain a list")
    if any(not isinstance(entry, dict) or not isinstance(entry.get("platform", {}), dict)
           for entry in index["manifests"]):
        raise ValueError("invalid platform metadata")
    candidates = [entry for entry in index.get("manifests", [])
                  if entry.get("platform", {}).get("os") == "linux"
                  and entry.get("platform", {}).get("architecture") == "amd64"]
    if len(candidates) != 1:
        raise ValueError("one unambiguous Linux/amd64 manifest required")
    value = candidates[0].get("digest", "")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
        raise ValueError("invalid platform manifest digest")
    return value


def resolve(image, transport=fetch):
    if not isinstance(image, str) or not re.fullmatch(r"swebench/[a-z0-9_.-]+:[a-zA-Z0-9_.-]+", image):
        raise ValueError("only explicitly declared public swebench Docker Hub images supported")
    repository, tag = image.rsplit(":", 1)
    query = urllib.parse.urlencode({"service": "registry.docker.io", "scope": "repository:" + repository + ":pull"})
    raw, _ = transport("https://auth.docker.io/token?" + query)
    response = json.loads(raw)
    if not isinstance(response, dict):
        raise ValueError("invalid registry token response")
    token = response.get("token")
    if not isinstance(token, str) or not token:
        raise ValueError("anonymous registry token unavailable")
    headers = {"Authorization": "Bearer " + token, "Accept": ACCEPT}
    raw, response_headers = transport("https://registry-1.docker.io/v2/" + repository + "/manifests/" + tag, headers)
    index_digest = digest(raw)
    reported = next((value for key, value in response_headers.items() if key.lower() == "docker-content-digest"), None)
    if reported is not None and reported != index_digest:
        raise ValueError("registry manifest digest mismatch")
    manifest = json.loads(raw)
    if not isinstance(manifest, dict):
        raise ValueError("manifest must be an object")
    selected = index_digest
    if "manifests" in manifest:
        selected = compatible_manifest(manifest)
        raw, _ = transport("https://registry-1.docker.io/v2/" + repository + "/manifests/" + selected, headers)
        if digest(raw) != selected:
            raise ValueError("platform manifest digest mismatch")
        manifest = json.loads(raw)
        if not isinstance(manifest, dict):
            raise ValueError("platform manifest must be an object")
    layers = manifest.get("layers")
    if not isinstance(layers, list) or not layers:
        raise ValueError("image layer metadata unavailable")
    total = 0
    for layer in layers:
        if not isinstance(layer, dict):
            raise ValueError("layer must be an object")
        size = layer.get("size")
        if type(size) is not int or size < 0 or not re.fullmatch(r"sha256:[0-9a-f]{64}", layer.get("digest", "")):
            raise ValueError("invalid layer metadata")
        total += size
    return {"image_tag": image, "index_digest": index_digest,
            "image_digest": selected, "immutable_image": repository + "@" + selected,
            "platform": "linux/amd64", "compressed_layer_bytes": total,
            "status": "manifest_locked_not_executed"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("prepared_results", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    # Prepared results are a bounded list, not an admission JSON document.
    with args.prepared_results.open("rb") as stream:
        raw = stream.read(16 * 1024 * 1024 + 1)
    if len(raw) > 16 * 1024 * 1024:
        raise ValueError("preparation ledger exceeds bound")
    rows = json.loads(raw)
    if not isinstance(rows, list) or not 1 <= len(rows) <= 50:
        raise ValueError("preparation ledger must contain 1–50 cases")
    if any(not isinstance(row, dict) or not isinstance(row.get("id"), str)
           or not isinstance(row.get("status"), str) for row in rows):
        raise ValueError("invalid preparation case metadata")
    with args.output.open("x") as output:
        result = []
        for row in rows:
            record = {"task": row["id"], "status": "unsupported_preparation"}
            if row["status"] == "prepared_not_validated":
                try:
                    record.update(resolve(row["image_tag"]))
                except (ValueError, OSError, KeyError, TypeError) as error:
                    record.update(status="manifest_unavailable", error=str(error))
            result.append(record)
            output.seek(0)
            output.write(json.dumps(result, indent=2) + "\n")
            output.truncate()
            output.flush()


if __name__ == "__main__":
    main()
