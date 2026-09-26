#!/usr/bin/env python3
"""Assembles dist/ from a built reforge.dll: the DLL, reforge.lua, the notices
file and SHA256SUMS. Usage: python scripts/make_dist.py <path to reforge.dll>"""

import hashlib
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    dll = Path(sys.argv[1])
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    shutil.copyfile(dll, dist / "reforge.dll")
    shutil.copyfile(ROOT / "lua" / "reforge.lua", dist / "reforge.lua")
    notices = (
        "Reforge (reforge.dll, reforge.lua) - https://github.com/Vansinnet/Reforge\n\n"
        + (ROOT / "LICENSE").read_text(encoding="utf-8")
        + "\n\n"
        + (ROOT / "THIRD_PARTY_NOTICES.md").read_text(encoding="utf-8")
    )
    (dist / "REFORGE_NOTICES.txt").write_text(notices, encoding="utf-8", newline="\n")
    lines = []
    for name in ("reforge.dll", "reforge.lua", "REFORGE_NOTICES.txt"):
        digest = hashlib.sha256((dist / name).read_bytes()).hexdigest()
        lines.append(f"{digest}  {name}")
    (dist / "SHA256SUMS").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
