# Changelog

## 0.1.0 (unreleased)

- First version: `reforge.dll` (ABI 1), `reforge.lua` (library 1), `reforge` CLI (init, add, add-virtual, add-virtual-dir, build, verify, sync, info).
- Offline test suites: DLL harness (real Win32 calls, run under Wine/Windows), Lua library with a fake DLL, LuaJIT FFI end-to-end, CLI.
- `reforge pack` Oodle-compresses stored (uncompressed) chunks in bundle payloads with the game's own Oodle library, verifying every chunk, and `reforge build` refuses bundles that still have them. Found in game: RainbowFlame's stored-chunk bundles crashed Darktide's DirectStorage reader ("Failed to decompress ... from package").
- Files that Wobin's Asset Redirect (for example Polychromatic) also serves are left to it and reported as `displaced`.
- A DMF hot reload no longer reports `restart_required` for unchanged files.
- In-game: RainbowBarrels (2 bundles + 1,098 material streams) served 1,100 of 1,100 files with the 0.1.0 development DLL (mingw build, SHA-256 `77aaa5af…`), with no errors.
