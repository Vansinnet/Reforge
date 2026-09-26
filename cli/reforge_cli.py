#!/usr/bin/env python3
"""reforge: create and maintain Darktide mods that replace game files with Reforge.

Standard library only (Python 3.9+). Run `reforge --help` or `reforge <command> --help`.

The mod keeps one hand-edited source file, `reforge.json`, next to its `.mod`
file. `reforge build` turns it into `scripts/mods/<Mod>/reforge_manifest.lua`
and copies the Reforge runtime (reforge.lua + bin/reforge.dll) into the mod.

The tool only ever reads the installed game.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bundle8  # noqa: E402

TOOL_VERSION = "0.1.0"
SCHEMA = 1
TOOL_ROOT = Path(__file__).resolve().parent.parent
DIST = TOOL_ROOT / "dist"
TEMPLATES = TOOL_ROOT / "templates" / "mod"
# The notices file must travel with every copy of reforge.dll (MinHook licence).
RUNTIME_FILES = {"reforge.lua": "lua", "reforge.dll": "dll", "REFORGE_NOTICES.txt": "notices"}
DEFAULT_GAME_DIRS = [
    r"C:\Program Files (x86)\Steam\steamapps\common\Warhammer 40,000 DARKTIDE",
    r"D:\SteamLibrary\steamapps\common\Warhammer 40,000 DARKTIDE",
    r"E:\SteamLibrary\steamapps\common\Warhammer 40,000 DARKTIDE",
]
MOD_NAME = re.compile(r"^[A-Za-z][A-Za-z0-9_]{1,63}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


class ToolError(Exception):
    pass


# Helpers ---------------------------------------------------------------------


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_rel(rel: str, prefix: str, what: str, lowercase: bool = False) -> None:
    bad = (
        not rel
        or not rel.startswith(prefix)
        or rel.endswith("/")
        or "\\" in rel
        or ":" in rel
        or "//" in rel
        or any(seg in (".", "..") for seg in rel.split("/"))
    )
    if bad:
        raise ToolError(f"{what} must be a path under {prefix} with forward slashes and no '..': {rel!r}")
    if lowercase and rel != rel.lower():
        raise ToolError(f"{what} must be lowercase: {rel!r}")


def stock_rel(value: str) -> str:
    value = value.strip().replace("\\", "/")
    if not value.startswith("bundle/"):
        value = "bundle/" + value
    check_rel(value, "bundle/", "stock path", lowercase=True)
    return value


def find_game(explicit: str | None) -> Path:
    candidates = [explicit] if explicit else [os.environ.get("DARKTIDE_DIR")] + DEFAULT_GAME_DIRS
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate)
        if (path / "bundle").is_dir() and (path / "binaries").is_dir():
            return path
    if explicit:
        raise ToolError(f"{explicit} is not a Darktide folder (needs bundle and binaries)")
    raise ToolError("Darktide folder not found; pass --game or set DARKTIDE_DIR")


def find_mod(path: str) -> tuple[Path, str]:
    root = Path(path).resolve()
    if not root.is_dir():
        raise ToolError(f"{root} is not a folder")
    name = root.name
    if not (root / f"{name}.mod").is_file():
        raise ToolError(f"{root} has no {name}.mod; the folder name must match the mod name")
    return root, name


def load_source(root: Path) -> dict:
    path = root / "reforge.json"
    if not path.is_file():
        return {"schema": SCHEMA, "redirects": [], "virtual_dirs": []}
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != SCHEMA:
        raise ToolError(f"{path} has schema {data.get('schema')!r}, this tool reads {SCHEMA}")
    data.setdefault("redirects", [])
    data.setdefault("virtual_dirs", [])
    return data


def save_source(root: Path, data: dict) -> None:
    data["redirects"].sort(key=lambda r: r["stock"])
    text = json.dumps(data, indent=2, ensure_ascii=False) + "\n"
    (root / "reforge.json").write_text(text, encoding="utf-8", newline="\n")


def lua_string(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def runtime_sources() -> dict[str, Path]:
    files = {name: DIST / name for name in RUNTIME_FILES}
    missing = [str(p) for p in files.values() if not p.is_file()]
    if missing:
        raise ToolError("Reforge runtime missing from the tool's dist folder: " + ", ".join(missing))
    return files


def runtime_targets(root: Path, name: str) -> dict[str, Path]:
    return {
        "reforge.lua": root / "scripts" / "mods" / name / "reforge.lua",
        "reforge.dll": root / "bin" / "reforge.dll",
        "REFORGE_NOTICES.txt": root / "bin" / "REFORGE_NOTICES.txt",
    }


def lib_version(lua_path: Path) -> int | None:
    if not lua_path.is_file():
        return None
    match = re.search(r"^local LIB_VERSION = (\d+)", lua_path.read_text(encoding="utf-8"), re.M)
    return int(match.group(1)) if match else None


# Commands --------------------------------------------------------------------


def cmd_init(args: argparse.Namespace) -> int:
    root = Path(args.path).resolve()
    name = root.name
    if not MOD_NAME.match(name):
        raise ToolError(f"mod name {name!r} must start with a letter and use only letters, digits and _")
    if root.exists() and any(root.iterdir()):
        raise ToolError(f"{root} already exists and is not empty")
    values = {
        "{{NAME}}": name,
        "{{AUTHOR}}": args.author or "",
        "{{DESCRIPTION}}": args.description or f"{name} replaces game files with Reforge.",
    }
    for template in sorted(TEMPLATES.rglob("*")):
        if template.is_dir():
            continue
        rel = template.relative_to(TEMPLATES).as_posix().replace("__NAME__", name)
        target = root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        text = template.read_text(encoding="utf-8")
        for key, value in values.items():
            text = text.replace(key, value)
        target.write_text(text, encoding="utf-8", newline="\n")
    (root / "payload").mkdir(exist_ok=True)
    save_source(root, {"schema": SCHEMA, "redirects": [], "virtual_dirs": []})
    sync_runtime(root, name, quiet=True)
    write_manifest(root, name, [])
    print(f"Created {root}")
    print("Next: reforge add <mod> <bundle file> --copy, edit the copy under payload/, then reforge build <mod>.")
    return 0


def cmd_add(args: argparse.Namespace) -> int:
    root, _ = find_mod(args.mod)
    game = find_game(args.game)
    stock = stock_rel(args.stock)
    stock_path = game.joinpath(*stock.split("/"))
    if not stock_path.is_file():
        raise ToolError(f"{stock} does not exist in {game}")
    file_rel = args.file or "payload/" + stock[len("bundle/"):]
    check_rel(file_rel, "payload/", "file")
    payload = root.joinpath(*file_rel.split("/"))
    if args.copy:
        if payload.exists():
            print(f"{file_rel} already exists; not overwritten")
        else:
            payload.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(stock_path, payload)
            print(f"Copied stock {stock} to {file_rel} as an editing base")
    data = load_source(root)
    entry = {"stock": stock, "file": file_rel, "sha256": sha256_file(stock_path)}
    if args.priority:
        entry["priority"] = args.priority
    if args.contract:
        entry["contract"] = args.contract
    data["redirects"] = [r for r in data["redirects"] if r["stock"] != stock] + [entry]
    save_source(root, data)
    print(f"Registered {stock} -> {file_rel} (stock sha256 {entry['sha256'][:16]}…)")
    return 0


def cmd_add_virtual(args: argparse.Namespace) -> int:
    root, _ = find_mod(args.mod)
    stock = stock_rel(args.stock)
    check_rel(args.file, "payload/", "file")
    data = load_source(root)
    entry = {"stock": stock, "file": args.file, "virtual": True}
    data["redirects"] = [r for r in data["redirects"] if r["stock"] != stock] + [entry]
    save_source(root, data)
    print(f"Registered new game path {stock} -> {args.file}")
    return 0


def cmd_add_virtual_dir(args: argparse.Namespace) -> int:
    root, _ = find_mod(args.mod)
    stock = stock_rel(args.stock).rstrip("/")
    check_rel(args.dir, "payload/", "dir")
    data = load_source(root)
    data["virtual_dirs"] = [d for d in data["virtual_dirs"] if d["stock"] != stock] + [{"stock": stock, "dir": args.dir}]
    save_source(root, data)
    print(f"Every file in {args.dir} will be served as {stock}/<name>")
    return 0


def expand(root: Path, data: dict) -> list[dict]:
    """All redirects, with virtual_dirs expanded, validated against the mod folder."""
    entries: dict[str, dict] = {}
    problems: list[str] = []

    def put(entry: dict) -> None:
        if entry["stock"] in entries:
            problems.append(f"{entry['stock']} is listed twice")
        entries[entry["stock"]] = entry

    for raw in data["redirects"]:
        try:
            stock = raw["stock"]
            check_rel(stock, "bundle/", "stock", lowercase=True)
            check_rel(raw["file"], "payload/", "file")
            if raw.get("virtual"):
                entry = {"stock": stock, "file": raw["file"], "virtual": True}
            else:
                sha = str(raw.get("sha256", "")).lower()
                if not HEX64.match(sha):
                    raise ToolError(f"{stock}: sha256 must be 64 hex digits")
                entry = {"stock": stock, "file": raw["file"], "sha256": sha}
            for key in ("priority", "contract"):
                if key in raw:
                    entry[key] = raw[key]
            if not root.joinpath(*raw["file"].split("/")).is_file():
                raise ToolError(f"{stock}: payload {raw['file']} is missing")
            put(entry)
        except (KeyError, ToolError) as exc:
            problems.append(str(exc))

    for vdir in data["virtual_dirs"]:
        try:
            stock_prefix = vdir["stock"].rstrip("/")
            check_rel(stock_prefix, "bundle/", "virtual_dirs stock", lowercase=True)
            check_rel(vdir["dir"], "payload/", "virtual_dirs dir")
            folder = root.joinpath(*vdir["dir"].split("/"))
            if not folder.is_dir():
                raise ToolError(f"virtual dir {vdir['dir']} is missing")
            for file in sorted(folder.rglob("*")):
                if file.is_file():
                    sub = file.relative_to(folder).as_posix()
                    if sub != sub.lower():
                        raise ToolError(f"{vdir['dir']}/{sub}: game paths are lowercase, rename the file")
                    put({"stock": f"{stock_prefix}/{sub}", "file": f"{vdir['dir']}/{sub}", "virtual": True})
        except (KeyError, ToolError) as exc:
            problems.append(str(exc))

    if problems:
        raise ToolError("reforge.json has problems:\n  " + "\n  ".join(problems))
    return [entries[key] for key in sorted(entries)]


def write_manifest(root: Path, name: str, entries: list[dict]) -> Path:
    lines = [
        "-- Generated by reforge build. Edit reforge.json instead.",
        "return {",
        f"\tschema = {SCHEMA},",
        "\tredirects = {",
    ]
    for entry in entries:
        parts = [f"stock = {lua_string(entry['stock'])}", f"file = {lua_string(entry['file'])}"]
        if entry.get("virtual"):
            parts.append("virtual = true")
        else:
            parts.append(f"sha256 = {lua_string(entry['sha256'])}")
        if "priority" in entry:
            parts.append(f"priority = {int(entry['priority'])}")
        if "contract" in entry:
            parts.append(f"contract = {lua_string(str(entry['contract']))}")
        lines.append("\t\t{ " + ", ".join(parts) + " },")
    lines += ["\t},", "}", ""]
    path = root / "scripts" / "mods" / name / "reforge_manifest.lua"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    return path


def sync_runtime(root: Path, name: str, quiet: bool = False) -> bool:
    sources = runtime_sources()
    targets = runtime_targets(root, name)
    changed = False
    shipped = lib_version(sources["reforge.lua"])
    present = lib_version(targets["reforge.lua"])
    if present is not None and shipped is not None and present > shipped:
        raise ToolError(f"{targets['reforge.lua']} is library {present}, newer than this tool's {shipped}; update the tool")
    for key, source in sources.items():
        target = targets[key]
        if target.is_file() and sha256_file(target) == sha256_file(source):
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        changed = True
        if not quiet:
            print(f"Updated {target.relative_to(root).as_posix()}")
    return changed


def bundle_payloads(root: Path, entries: list[dict]) -> list[tuple[dict, Path]]:
    """Payload files that are format-8 bundles."""
    found = []
    for entry in entries:
        path = root.joinpath(*entry["file"].split("/"))
        with path.open("rb") as handle:
            if bundle8.is_bundle(handle.read(8)):
                found.append((entry, path))
    return found


def stored_problems(root: Path, entries: list[dict]) -> list[str]:
    problems = []
    for entry, path in bundle_payloads(root, entries):
        try:
            count = bundle8.stored_chunks(path.read_bytes())
        except bundle8.BundleError as exc:
            problems.append(f"{entry['file']}: {exc}")
            continue
        if count:
            problems.append(f"{entry['file']}: {count} uncompressed chunk(s)")
    return problems


def cmd_build(args: argparse.Namespace) -> int:
    root, name = find_mod(args.mod)
    data = load_source(root)
    entries = expand(root, data)
    problems = stored_problems(root, entries)
    if problems:
        raise ToolError(
            "bundles with uncompressed (stored) chunks crash Darktide's DirectStorage reader "
            "(\"Failed to decompress ... from package\"). Run `reforge pack` first:\n  " + "\n  ".join(problems)
        )
    if args.game:
        failures = verify_entries(find_game(args.game), entries)
        if failures:
            raise ToolError("stock files differ from reforge.json:\n  " + "\n  ".join(failures))
    path = write_manifest(root, name, entries)
    sync_runtime(root, name)
    virtual = sum(1 for e in entries if e.get("virtual"))
    print(f"Wrote {path.relative_to(root).as_posix()}: {len(entries) - virtual} replaced, {virtual} new game paths")
    return 0


def verify_entries(game: Path, entries: list[dict]) -> list[str]:
    failures = []
    for entry in entries:
        path = game.joinpath(*entry["stock"].split("/"))
        if entry.get("virtual"):
            if path.exists():
                failures.append(f"{entry['stock']}: now exists in the game; a virtual path must not")
            continue
        if not path.is_file():
            failures.append(f"{entry['stock']}: missing from the game")
            continue
        actual = sha256_file(path)
        if actual != entry["sha256"]:
            failures.append(f"{entry['stock']}: sha256 {actual}, expected {entry['sha256']} (game updated?)")
    return failures


def cmd_pack(args: argparse.Namespace) -> int:
    root, _ = find_mod(args.mod)
    entries = expand(root, load_source(root))
    targets = []
    for entry, path in bundle_payloads(root, entries):
        if bundle8.stored_chunks(path.read_bytes()):
            targets.append((entry, path))
    if not targets:
        print("No bundle payload has uncompressed chunks")
        return 0
    dll = Path(args.oodle) if args.oodle else find_game(args.game) / "binaries" / "oo2core_9_win64.dll"
    if not dll.is_file():
        raise ToolError(f"Oodle library not found: {dll}")
    try:
        oodle = bundle8.Oodle(dll)
        for entry, path in targets:
            before = path.read_bytes()
            after = bundle8.pack(before, oodle, args.level)
            tmp = path.with_name(path.name + ".reforge-pack.tmp")
            tmp.write_bytes(after)
            os.replace(tmp, path)
            print(f"Packed {entry['file']}: {len(before):,} -> {len(after):,} bytes")
    except bundle8.BundleError as exc:
        raise ToolError(str(exc)) from exc
    return 0


def cmd_verify(args: argparse.Namespace) -> int:
    root, _ = find_mod(args.mod)
    game = find_game(args.game)
    entries = expand(root, load_source(root))
    failures = verify_entries(game, entries)
    for failure in failures:
        print(failure)
    print(f"{len(entries) - len(failures)} of {len(entries)} redirects match {game}")
    return 1 if failures else 0


def cmd_sync(args: argparse.Namespace) -> int:
    root, name = find_mod(args.mod)
    if not sync_runtime(root, name):
        print("Runtime already up to date")
    return 0


def cmd_info(args: argparse.Namespace) -> int:
    print(f"reforge tool {TOOL_VERSION}")
    for name in RUNTIME_FILES:
        path = DIST / name
        state = sha256_file(path) if path.is_file() else "missing"
        print(f"  dist/{name}: {state}")
    lua = lib_version(DIST / "reforge.lua")
    print(f"  Lua library version: {lua}")
    try:
        print(f"  Darktide: {find_game(args.game)}")
    except ToolError as exc:
        print(f"  Darktide: {exc}")
    return 0


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="reforge", description=__doc__.split("\n\n")[0])
    p.add_argument("--version", action="version", version=f"reforge {TOOL_VERSION}")
    sub = p.add_subparsers(dest="command", required=True)

    s = sub.add_parser("init", help="create a new mod wired for Reforge")
    s.add_argument("path", help="new mod folder; its name is the mod name")
    s.add_argument("--author")
    s.add_argument("--description")
    s.set_defaults(func=cmd_init)

    s = sub.add_parser("add", help="replace an existing game file (records its sha256)")
    s.add_argument("mod")
    s.add_argument("stock", help="bundle/<file> or just the bundle file name")
    s.add_argument("--file", help="payload path in the mod (default payload/<stock name>)")
    s.add_argument("--copy", action="store_true", help="copy the stock file to the payload path as an editing base")
    s.add_argument("--priority", type=int, default=0)
    s.add_argument("--contract", help="mods with the same contract count as compatible")
    s.add_argument("--game")
    s.set_defaults(func=cmd_add)

    s = sub.add_parser("add-virtual", help="serve a file at a game path that does not exist yet")
    s.add_argument("mod")
    s.add_argument("stock", help="bundle/<path>")
    s.add_argument("file", help="payload/<path>")
    s.set_defaults(func=cmd_add_virtual)

    s = sub.add_parser("add-virtual-dir", help="serve every file in a payload folder under a new game folder")
    s.add_argument("mod")
    s.add_argument("stock", help="bundle/<folder>")
    s.add_argument("dir", help="payload/<folder>")
    s.set_defaults(func=cmd_add_virtual_dir)

    s = sub.add_parser("build", help="validate reforge.json, write the Lua manifest and sync the runtime")
    s.add_argument("mod")
    s.add_argument("--game", help="also check every stock sha256 against this game folder")
    s.set_defaults(func=cmd_build)

    s = sub.add_parser("pack", help="Oodle-compress uncompressed chunks in the mod's bundle payloads (Windows)")
    s.add_argument("mod")
    s.add_argument("--game", help="Darktide folder; its binaries/oo2core_9_win64.dll is used")
    s.add_argument("--oodle", help="explicit path to oo2core_9_win64.dll")
    s.add_argument("--level", type=int, default=bundle8.LEVEL_NORMAL, help="Oodle level (default 4, Normal)")
    s.set_defaults(func=cmd_pack)

    s = sub.add_parser("verify", help="check that stock files still match after a game update")
    s.add_argument("mod")
    s.add_argument("--game")
    s.set_defaults(func=cmd_verify)

    s = sub.add_parser("sync", help="copy this tool's reforge.lua and reforge.dll into the mod")
    s.add_argument("mod")
    s.set_defaults(func=cmd_sync)

    s = sub.add_parser("info", help="show tool, runtime and game folder")
    s.add_argument("--game")
    s.set_defaults(func=cmd_info)
    return p


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        return args.func(args)
    except ToolError as exc:
        print(f"reforge: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
