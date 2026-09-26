# Reforge

Reforge lets a Warhammer 40,000: Darktide mod replace the game's own files,
such as particle effects, materials and shader programs, by shipping edited
copies inside the mod folder. Players install the mod like any other DMF mod
by dropping it into `mods/`. No installer runs, and no game file is changed on
disk. Uninstalling means deleting the mod folder.

It has three parts:

| Part | What it is |
|---|---|
| `reforge.dll` | A small native library (Rust). While the game runs, it serves read-only opens of registered `bundle/…` files from files in `mods/…`. |
| `reforge.lua` | The Lua library that mods ship. It loads one shared copy of the DLL, lets several mods coexist, and reports a state for each replaced file. |
| `reforge` CLI | A tool for mod authors. It scaffolds a mod, records stock hashes, generates the Lua manifest and checks mods after game updates. |

## How it works

1. DMF loads the mod. The mod's script calls `reforge.register_manifest(...)`.
2. In `on_all_mods_loaded`, the mod calls `reforge.commit()`. The library loads
   `bin/reforge.dll` through `Mods.lua.ffi` (LuaJIT FFI) and installs hooks on
   `CreateFileW`, `CreateFileA` and `CreateFile2`, in this process only.
3. For every registration, the DLL checks that the stock file still has the
   expected SHA-256. When it does, later read-only opens of that file get a
   handle to the mod's copy. When the game has been patched and the hash no
   longer matches, the file stays stock and the mod is told why.

### What the DLL will and won't do

- It redirects only read-only opens under `<game>/bundle/`. Opens for writing
  or deleting always go to the real file.
- Replacements must live under `<game>/mods/`. Paths with `..`, drive letters
  or backslashes are refused.
- It makes no network calls, writes no files, and patches no memory apart
  from the three hooks.
- Only one copy hooks a process. If several mods ship `reforge.dll`, the
  first loaded copy is shared by all of them.
- Every export catches panics, so an internal bug falls back to stock files
  instead of crashing the game.

## For mod authors

The steps below use Windows, Python 3.9+ and the `reforge.cmd` wrapper in this
folder.

```bat
reforge init path\to\mods\MyShaderMod --author "Me"
reforge add path\to\mods\MyShaderMod 98bb14b1d247a0c8 --copy
rem edit path\to\mods\MyShaderMod\payload\98bb14b1d247a0c8 with your own tools
reforge pack path\to\mods\MyShaderMod --game "C:\...\Warhammer 40,000 DARKTIDE"
reforge build path\to\mods\MyShaderMod --game "C:\...\Warhammer 40,000 DARKTIDE"
```

- `add` records the stock file's SHA-256 in `reforge.json`. With `--copy`, it
  also copies the stock file into `payload/` as a starting point for editing.
- `add-virtual` and `add-virtual-dir` serve files at game paths that do not
  exist yet. Use them for extra resources that a replaced bundle refers to.
- `pack` Oodle-compresses bundle payloads whose 512 KiB chunks are stored
  uncompressed. Use it after writing a bundle with your own tools. The game's
  DirectStorage reader crashes on stored chunks with "Failed to decompress ...
  from package". `pack` uses the Oodle library installed with the game, which
  is not redistributable, so it needs Windows. It checks every chunk by
  decompressing it again.
- `build` refuses a bundle payload that still has stored chunks, then
  validates everything and writes
  `scripts/mods/<Mod>/reforge_manifest.lua`. It also copies the current
  `reforge.lua`, `bin/reforge.dll` and `bin/REFORGE_NOTICES.txt` into the mod.
- After a game update, `verify` shows which stock files changed.

See [docs/authoring.md](docs/authoring.md) for the Lua API, file states and
coexistence rules, and [docs/ABI.md](docs/ABI.md) for the C ABI.

## File states

| State | Meaning |
|---|---|
| `active` | This mod's file is being served. |
| `shared` | Another mod won, but it serves a byte-identical file. |
| `compatible` | Another mod won with the same `contract` string. |
| `displaced` | Another mod's different file is served. |
| `refused` | The stock file changed (game update), the payload is missing, or a path is invalid. See `reforge.reason(handle)`. |
| `restart_required` | The game already opened this file this session with other contents. The change applies after a restart. |
| `unavailable` | `reforge.dll` could not be loaded. |
| `pending` | `commit()` has not run yet. |

In game, `/reforge` lists every replaced file. `/reforge on|off` toggles
redirects for files opened after that point. `/reforge trace on|off|dump`
writes a file trace to the log.

## Limits

- Files the game opened before `reforge.dll` was installed cannot be detected
  after the fact. Most effect and material bundles load with missions, after
  mods load. A file loaded at boot needs a restart to be picked up.
- **Early-loaded bundles must keep the stock layout exactly.** Darktide
  reads some bundles, such as the weapons of the character in the menu,
  before any mod runs, and loads them later with the stock chunk table. A
  replacement with a larger chunk crashes with "Failed to decompress ... from
  package". A smaller one crashes with read error `0x89240007`, which means
  end of file. This was observed in game on 2026-09-26. `reforge pack --game`
  recompresses each chunk and pads it to the stock chunk size (Oodle ignores
  trailing bytes), which works for edits that keep the stock record list. New
  resources cannot be added to such a bundle. Put them in a package that the
  game never loads on its own, replace that package's bundle (mark it
  `"late_load": true`), and load it from the mod after `reforge.commit()`.
  See `docs/authoring.md`. Bundles the game first loads in missions may
  differ from stock; mark them `late_load` too.
- Game updates change stock hashes. Affected files fall back to stock until
  the mod is rebuilt against the new files.
- Polychromatic and some other mods ship Wobin's Asset Redirect, which is a
  separate hooking DLL. Two hooks serving the same file would race, so
  Reforge leaves any file that Asset Redirect also registers to Asset
  Redirect. That file reports `displaced`, and `reforge.winner(handle)` names
  the other mod. Files only one of them replaces are unaffected.

## Building

- **Windows (MSVC):** `powershell -File scripts\build.ps1 -Test`. This builds,
  runs the harness and refreshes `dist/`.
- **Linux cross-build + Wine:** `scripts/build-cross.sh`.
- **Tests:**
  - `luajit lua/tests/test_reforge.lua`
  - `python -m unittest discover -s cli/tests`
  - `lua/tests/integration.lua`, which runs real LuaJIT FFI against the real DLL.

Release builds come only from GitHub Actions. Each tag produces a draft
release with build provenance you can check with
`gh attestation verify reforge.dll --repo Vansinnet/Reforge`. The DLL is
unsigned. Antivirus heuristics may flag any unsigned DLL that hooks
`CreateFileW`. Compare the file against the release's `SHA256SUMS` and
attestation, and report false positives to your antivirus vendor. Never
disable scanning.

## Credits

Thanks to Wobin for the original Asset Redirect DLL for Darktide. His work
showed that mods could serve replacement game resources from their own folders
and inspired Reforge. Reforge is an independent implementation; it does not
include Wobin's DLL or source code.

## Licence

MIT, see [LICENSE](LICENSE). The DLL includes MinHook (BSD-2-Clause); see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Mods that ship
`reforge.dll` must ship `REFORGE_NOTICES.txt` with it, and `reforge build`
does that automatically.

Reforge is a community tool. It is not affiliated with Fatshark or Games
Workshop.
