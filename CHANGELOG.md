# Changelog

## 0.1.0 (unreleased)

- First version: `reforge.dll` (ABI 1), `reforge.lua` (library 1), `reforge` CLI (init, add, add-virtual, add-virtual-dir, build, verify, sync, info).
- Offline test suites: DLL harness (real Win32 calls, run under Wine/Windows), Lua library with a fake DLL, LuaJIT FFI end-to-end, CLI.
- `reforge pack` Oodle-compresses stored (uncompressed) chunks in bundle payloads with the game's own Oodle library, verifying every chunk, and `reforge build` refuses bundles that still have them. Found in game: RainbowFlame's stored-chunk bundles crashed Darktide's DirectStorage reader ("Failed to decompress ... from package").
- Redirected files report the replacement's size and attributes (`GetFileAttributes(Ex)W/A`, `FindFirstFileExW`), and a stock file already open in the game is refused and stays stock.
- `reforge build --game` / `reforge verify` warn about replacement bundles larger than stock: in game, Darktide's DirectStorage reader loaded such a bundle (RainbowFlame's Inferno staff effect `3d487cca`) with the stock size and crashed. `reforge pack --game` tries stronger compression to fit.
- Files that Wobin's Asset Redirect (for example Polychromatic) also serves are left to it and reported as `displaced`.
- A DMF hot reload no longer reports `restart_required` for unchanged files.
- In-game: RainbowBarrels (2 bundles + 1,098 material streams) served 1,100 of 1,100 files with the 0.1.0 development DLL (mingw build, SHA-256 `77aaa5af…`), with no errors.
