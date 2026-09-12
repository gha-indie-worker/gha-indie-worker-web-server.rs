#!/usr/bin/env python3
"""Vendor htmx into assets/, pinned by SHA-384.

Why this exists
---------------
htmx is *not* loaded from a CDN at run time. A `<script src="https://unpkg.com/...">` makes every
page in the product depend on a third party's account security and on a network path we do not
control; an `integrity` attribute narrows that to a denial of service rather than a compromise,
but the bytes are still whatever the registry decides to serve. So the file is downloaded once,
verified, committed, embedded in the binary by `build.rs`, and served from our own origin under
`script-src 'self'`.

First run writes `assets/htmx.min.js.sha384` from what it downloaded (trust on first use, by a
human, once). Every run after that — including CI, which runs `--check` — verifies against the
committed pin and fails if the bytes have changed. Bumping the version is therefore a deliberate
two-line diff a reviewer can see, not something that happens on its own.

    python3 scripts/vendor-htmx.py                 # download and verify (or pin, first time)
    python3 scripts/vendor-htmx.py --check         # verify what is committed; download nothing
    python3 scripts/vendor-htmx.py --version 2.0.6 # bump, and record the new pin
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
SCRIPT = ASSETS / "htmx.min.js"
PIN = ASSETS / "htmx.min.js.sha384"
VERSION_FILE = ASSETS / "htmx.version"

# Only ever this host, and only ever over TLS. The file is checked against the committed pin
# immediately afterwards, so a compromised registry can at worst stop a bump, not land one.
SOURCE = "https://unpkg.com/htmx.org@{version}/dist/htmx.min.js"


def integrity(data: bytes) -> str:
    return "sha384-" + base64.b64encode(hashlib.sha384(data).digest()).decode("ascii")


def read_version(explicit: str | None) -> str:
    if explicit:
        return explicit.strip()
    if VERSION_FILE.is_file():
        return VERSION_FILE.read_text(encoding="utf-8").strip()
    raise SystemExit(
        "no version: pass --version <x.y.z> (assets/htmx.version records the one in use)"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", help="htmx release to vendor")
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed file against the committed pin; download nothing",
    )
    args = parser.parse_args()

    if args.check:
        if not SCRIPT.is_file() or not PIN.is_file():
            print(
                "htmx is not vendored: run `python3 scripts/vendor-htmx.py`.\n"
                "The server builds and serves without it — pages lose fragment swaps — so this is\n"
                "a warning in a development checkout and an error in CI.",
                file=sys.stderr,
            )
            return 1
        actual = integrity(SCRIPT.read_bytes())
        expected = PIN.read_text(encoding="utf-8").strip()
        if actual != expected:
            print(f"assets/htmx.min.js does not match its pin\n  pin:    {expected}\n  actual: {actual}", file=sys.stderr)
            return 1
        print(f"htmx {read_version(None)} matches its pin ({expected})")
        return 0

    version = read_version(args.version)
    url = SOURCE.format(version=version)
    print(f"fetching {url}")
    with urllib.request.urlopen(url, timeout=60) as response:  # noqa: S310 - literal https host
        payload = response.read()
    if not payload or b"htmx" not in payload[:4096]:
        print("that does not look like htmx; refusing to vendor it", file=sys.stderr)
        return 1

    actual = integrity(payload)
    if PIN.is_file():
        expected = PIN.read_text(encoding="utf-8").strip()
        recorded = read_version(None)
        if expected != actual and recorded == version:
            print(
                f"htmx {version} no longer hashes to the committed pin.\n"
                f"  pin:    {expected}\n  actual: {actual}\n"
                "A published release changing bytes is exactly what the pin is for. Do not\n"
                "overwrite it without finding out why.",
                file=sys.stderr,
            )
            return 1

    ASSETS.mkdir(parents=True, exist_ok=True)
    SCRIPT.write_bytes(payload)
    PIN.write_text(actual + "\n", encoding="utf-8")
    VERSION_FILE.write_text(version + "\n", encoding="utf-8")
    print(f"vendored htmx {version} ({len(payload)} bytes)\n  {actual}")
    print("commit assets/htmx.min.js, assets/htmx.min.js.sha384 and assets/htmx.version together.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
