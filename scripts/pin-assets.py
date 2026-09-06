#!/usr/bin/env python3
"""Recompute the SHA-384 pins for the assets this repository owns.

`assets/app.css` is embedded in the binary by `include_str!`, so the only thing that can drift is
the committed pin next to it — and a stale pin is caught by `assets::tests::
the_stylesheet_matches_its_committed_pin` rather than by a browser. Run this after editing the
stylesheet.

    python3 scripts/pin-assets.py            # rewrite the pins
    python3 scripts/pin-assets.py --check    # fail if any pin is stale (CI)
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OWNED = ["assets/app.css", "assets/tail.js"]


def integrity(data: bytes) -> str:
    return "sha384-" + base64.b64encode(hashlib.sha384(data).digest()).decode("ascii")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    stale: list[str] = []
    for name in OWNED:
        source = ROOT / name
        if not source.is_file():
            print(f"missing {name}", file=sys.stderr)
            return 1
        pin = ROOT / (name + ".sha384")
        actual = integrity(source.read_bytes())
        current = pin.read_text(encoding="utf-8").strip() if pin.is_file() else ""
        if current == actual:
            continue
        if args.check:
            stale.append(f"{name}: pin {current or '(absent)'} != {actual}")
        else:
            pin.write_text(actual + "\n", encoding="utf-8")
            print(f"pinned {name} {actual}")

    if stale:
        print("stale asset pins; run `python3 scripts/pin-assets.py`:", file=sys.stderr)
        for line in stale:
            print(f"  {line}", file=sys.stderr)
        return 1
    if args.check:
        print("asset pins are current")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
