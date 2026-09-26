# reforge.dll C ABI (ABI 1)

All exports use the platform C calling convention and are safe to call from
LuaJIT FFI.

**Strings**
- Input strings are NUL-terminated UTF-8.
- All paths are relative to the game folder and use forward slashes.

**Game folder**
- It is the nearest ancestor of `reforge.dll` that contains both `bundle` and
  `binaries`.

**Output buffers**
- *Truncating* outputs are always NUL-terminated.
- *Sized* outputs work like `snprintf`: the return value is the full length.
  The text is written only when `cap > length`.

```c
int Reforge_Abi(void);                         // 1. Changes only on incompatible changes.
int Reforge_Version(char* out, int cap);       // 1; writes "0.1.0" (truncating).
int Reforge_Install(char* err, int errlen);    // 1 ok (idempotent); 0 + message.
int Reforge_Add(const char* stock, const char* replacement,
                const char* expect_stock_sha256, char* err, int errlen);   // 1 / 0 + message
int Reforge_AddNew(const char* path, const char* replacement,
                   char* err, int errlen);     // 1 / 0 + message
int Reforge_Remove(const char* stock);         // 1 removed, 0 not registered
int Reforge_Clear(void);                       // number removed
int Reforge_Opens(const char* stock);          // read-only opens since install, -1 bad arg
int Reforge_HashFile(const char* rel, char* out, int cap);   // 1 + hex / 0 + message
int Reforge_SetEnabled(int on);                // previous state (0/1)
int Reforge_Stats(char* out, int cap);         // sized JSON, -1 on error
int Reforge_TraceEnable(int on);               // previous state; turning on clears the buffer
int Reforge_TraceDump(char* out, int cap);     // sized text, one event per line
```

## Rules

| Call | Checks |
|---|---|
| `Reforge_Install` | Refuses when the DLL is not inside a game folder, or when another copy of `reforge.dll` already hooks the process. |
| `Reforge_Add` | `stock` is lowercase and starts with `bundle/`. `replacement` starts with `mods/`. Neither contains `..`, `\`, `:` or `//`. The stock file exists and its SHA-256 equals `expect_stock_sha256` (case-insensitive). The replacement exists. Registering the same stock again replaces the earlier entry. |
| `Reforge_AddNew` | Same path rules as `Reforge_Add`, but the stock path must not exist. It may be in a folder that does not exist. |
| `Reforge_HashFile` | Accepts `bundle/…` or `mods/…` and always hashes the real file on disk, never a replacement. |

**Which opens are redirected:**
- The access mask has no write, delete or ACL rights.
- The disposition is `OPEN_EXISTING` or `OPEN_ALWAYS`.
- `FILE_FLAG_DELETE_ON_CLOSE` is not set.

**Path matching:** the requested path is resolved with `GetFullPathNameW`,
stripped of `\\?\`, lowercased, and compared against both the long and the 8.3
short spelling of the game folder.

**Fallback:** if the replacement cannot be opened, the stock file is opened
instead, and the event is traced.

## Stats JSON

```json
{"abi":1,"version":"0.1.0","installed":true,"enabled":true,
 "root":"C:\\...\\Warhammer 40,000 DARKTIDE","bundle_files_opened":812,
 "entries":[{"key":"bundle/98bb14b1d247a0c8","replacement":"mods/MyMod/payload/98bb14b1d247a0c8",
             "virtual":false,"sha256":"…","served":3,"opens":3}]}
```
