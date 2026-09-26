//! Offline test harness for reforge.dll.
//!
//! Builds a fake Darktide folder (bundle/, binaries/, mods/), loads the DLL
//! the way a mod does (relative to binaries/), and checks every redirect rule
//! through real CreateFileW/CreateFileA/CreateFile2 calls.
//!
//! Usage: reforge-harness <path to reforge.dll>

use std::ffi::{CString, c_char, c_int};
use std::fs;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, FreeLibrary, HANDLE, HMODULE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFile2, CreateFileA, CreateFileW, FILE_ATTRIBUTE_NORMAL, FindClose, FindFirstFileExW, FindFirstFileW, GetFileAttributesA,
    GetFileAttributesExA, GetFileAttributesExW, GetFileAttributesW, GetFileExInfoStandard, GetShortPathNameW,
    INVALID_FILE_ATTRIBUTES, WIN32_FILE_ATTRIBUTE_DATA, WIN32_FIND_DATAW, FILE_SHARE_READ, OPEN_EXISTING, ReadFile,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

const GENERIC_READ: u32 = 0x8000_0000;

type Abi = unsafe extern "C" fn() -> c_int;
type Text = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;
type Install = unsafe extern "C" fn(*mut c_char, c_int) -> c_int;
type Add = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *mut c_char, c_int) -> c_int;
type AddNew = unsafe extern "C" fn(*const c_char, *const c_char, *mut c_char, c_int) -> c_int;
type ByPath = unsafe extern "C" fn(*const c_char) -> c_int;
type Clear = unsafe extern "C" fn() -> c_int;
type Hash = unsafe extern "C" fn(*const c_char, *mut c_char, c_int) -> c_int;
type Toggle = unsafe extern "C" fn(c_int) -> c_int;

struct Lib {
    module: HMODULE,
    abi: Abi,
    version: Text,
    install: Install,
    add: Add,
    add_new: AddNew,
    remove: ByPath,
    clear: Clear,
    opens: ByPath,
    hash: Hash,
    set_enabled: Toggle,
    stats: Text,
    trace_enable: Toggle,
    trace_dump: Text,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

unsafe fn sym<T>(module: HMODULE, name: &str) -> T {
    let cname = c(name);
    let p = unsafe { GetProcAddress(module, cname.as_ptr() as *const u8) };
    let p = p.unwrap_or_else(|| panic!("missing export {name}"));
    unsafe { std::mem::transmute_copy(&p) }
}

fn load(path: &str) -> Lib {
    let module = unsafe { LoadLibraryW(wide(path).as_ptr()) };
    assert!(!module.is_null(), "LoadLibraryW({path}) failed");
    unsafe {
        Lib {
            module,
            abi: sym(module, "Reforge_Abi"),
            version: sym(module, "Reforge_Version"),
            install: sym(module, "Reforge_Install"),
            add: sym(module, "Reforge_Add"),
            add_new: sym(module, "Reforge_AddNew"),
            remove: sym(module, "Reforge_Remove"),
            clear: sym(module, "Reforge_Clear"),
            opens: sym(module, "Reforge_Opens"),
            hash: sym(module, "Reforge_HashFile"),
            set_enabled: sym(module, "Reforge_SetEnabled"),
            stats: sym(module, "Reforge_Stats"),
            trace_enable: sym(module, "Reforge_TraceEnable"),
            trace_dump: sym(module, "Reforge_TraceDump"),
        }
    }
}

impl Lib {
    fn install(&self) -> Result<(), String> {
        let mut err = [0 as c_char; 1024];
        let ok = unsafe { (self.install)(err.as_mut_ptr(), err.len() as c_int) };
        if ok == 1 { Ok(()) } else { Err(text(&err)) }
    }

    fn add(&self, stock: &str, replacement: &str, sha: &str) -> Result<(), String> {
        let mut err = [0 as c_char; 1024];
        let ok = unsafe {
            (self.add)(c(stock).as_ptr(), c(replacement).as_ptr(), c(sha).as_ptr(), err.as_mut_ptr(), 1024)
        };
        if ok == 1 { Ok(()) } else { Err(text(&err)) }
    }

    fn add_new(&self, stock: &str, replacement: &str) -> Result<(), String> {
        let mut err = [0 as c_char; 1024];
        let ok = unsafe { (self.add_new)(c(stock).as_ptr(), c(replacement).as_ptr(), err.as_mut_ptr(), 1024) };
        if ok == 1 { Ok(()) } else { Err(text(&err)) }
    }

    fn opens(&self, stock: &str) -> i32 {
        unsafe { (self.opens)(c(stock).as_ptr()) }
    }

    fn hash(&self, rel: &str) -> Result<String, String> {
        let mut out = [0 as c_char; 1024];
        let ok = unsafe { (self.hash)(c(rel).as_ptr(), out.as_mut_ptr(), 1024) };
        if ok == 1 { Ok(text(&out)) } else { Err(text(&out)) }
    }

    fn sized(&self, f: Text) -> String {
        let need = unsafe { f(null_mut(), 0) };
        assert!(need >= 0);
        let mut buf = vec![0 as c_char; need as usize + 1];
        let got = unsafe { f(buf.as_mut_ptr(), buf.len() as c_int) };
        assert_eq!(got, need);
        text(&buf)
    }
}

fn text(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf.iter().take_while(|&&b| b != 0).map(|&b| b as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn read_handle(handle: HANDLE) -> Vec<u8> {
    assert!(handle != INVALID_HANDLE_VALUE, "open failed");
    let mut buf = vec![0u8; 256];
    let mut read = 0u32;
    let ok = unsafe { ReadFile(handle, buf.as_mut_ptr(), buf.len() as u32, &mut read, null_mut()) };
    unsafe { CloseHandle(handle) };
    assert!(ok != 0, "ReadFile failed");
    buf.truncate(read as usize);
    buf
}

fn read_w(path: &[u16]) -> Vec<u8> {
    let h = unsafe {
        CreateFileW(path.as_ptr(), GENERIC_READ, FILE_SHARE_READ, null(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    read_handle(h)
}

fn read_a(path: &str) -> Vec<u8> {
    let p = c(path);
    let h = unsafe {
        CreateFileA(p.as_ptr() as *const u8, GENERIC_READ, FILE_SHARE_READ, null(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    read_handle(h)
}

fn read_2(path: &str) -> Vec<u8> {
    let h = unsafe { CreateFile2(wide(path).as_ptr(), GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING, null()) };
    read_handle(h)
}

fn read_std(path: &str) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn read_std_rw(path: &str) -> Vec<u8> {
    let mut f = fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
    let mut v = Vec::new();
    f.read_to_end(&mut v).unwrap();
    v
}

fn short_path(p: &Path) -> String {
    let input = wide_path(p);
    let mut buf = vec![0u16; 1024];
    let n = unsafe { GetShortPathNameW(input.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return p.to_str().unwrap().to_owned();
    }
    String::from_utf16_lossy(&buf[..n])
}

fn sha256(data: &[u8]) -> String {
    // Tiny independent SHA-256 so the harness does not trust the DLL's hash.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

struct Checks {
    passed: u32,
    failed: u32,
}

impl Checks {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Debug) {
        if ok {
            self.passed += 1;
            println!("ok   {name}");
        } else {
            self.failed += 1;
            println!("FAIL {name}: {detail:?}");
        }
    }
}

fn main() {
    let dll = PathBuf::from(std::env::args().nth(1).expect("usage: reforge-harness <reforge.dll>"));
    let dll = fs::canonicalize(&dll).unwrap_or(dll);
    let base = std::env::temp_dir().join(format!("reforge-harness-{}", std::process::id()));
    let game = base.join("Warhammer 40,000 DARKTIDE");
    for dir in ["bundle", "binaries", "mods/alpha/bin", "mods/alpha/payload", "mods/beta/bin"] {
        fs::create_dir_all(game.join(dir)).unwrap();
    }
    fs::create_dir_all(base.join("outside")).unwrap();
    fs::write(game.join("bundle/aaaa1111"), b"STOCK-A").unwrap();
    fs::write(game.join("bundle/bbbb2222"), b"STOCK-B").unwrap();
    fs::write(game.join("binaries/settings.ini"), b"SETTINGS").unwrap();
    fs::write(game.join("mods/alpha/payload/a.bin"), b"MOD-A").unwrap();
    fs::write(game.join("mods/alpha/payload/b.bin"), b"MOD-B").unwrap();
    fs::write(game.join("mods/alpha/payload/new.bin"), b"NEW").unwrap();
    for target in ["outside/reforge.dll", "Warhammer 40,000 DARKTIDE/mods/alpha/bin/reforge.dll", "Warhammer 40,000 DARKTIDE/mods/beta/bin/reforge.dll"] {
        fs::copy(&dll, base.join(target)).unwrap();
    }
    std::env::set_current_dir(game.join("binaries")).unwrap();

    let sha_a = sha256(b"STOCK-A");
    let sha_b = sha256(b"STOCK-B");
    let mut t = Checks { passed: 0, failed: 0 };

    // A copy outside the game folder must refuse to install and hook nothing.
    let outside = load(base.join("outside/reforge.dll").to_str().unwrap());
    let r = outside.install();
    t.check("outside copy refuses install", r.as_ref().is_err_and(|e| e.contains("bundle and binaries")), &r);
    unsafe { FreeLibrary(outside.module) };

    // Load exactly like Lua: relative to binaries/.
    let lib = load("..\\mods\\alpha\\bin\\reforge.dll");
    t.check("abi is 1", unsafe { (lib.abi)() } == 1, ());
    let version = lib.sized(lib.version);
    t.check("version reported", !version.is_empty(), &version);
    let r = lib.add("bundle/aaaa1111", "mods/alpha/payload/a.bin", &sha_a);
    t.check("add before install refused", r.as_ref().is_err_and(|e| e.contains("Install")), &r);
    let r = lib.install();
    t.check("install", r.is_ok(), &r);
    t.check("install is idempotent", lib.install().is_ok(), ());

    let v = read_std("../bundle/aaaa1111");
    t.check("stock served before registration", v == b"STOCK-A", &v);
    t.check("opens counted", lib.opens("bundle/aaaa1111") == 1, lib.opens("bundle/aaaa1111"));
    t.check("unopened file has zero opens", lib.opens("bundle/bbbb2222") == 0, ());

    let r = lib.add("bundle/aaaa1111", "mods/alpha/payload/a.bin", &sha_a.to_uppercase());
    t.check("add with correct sha", r.is_ok(), &r);
    let v = read_std("../bundle/aaaa1111");
    t.check("std::fs relative path redirected", v == b"MOD-A", &v);
    let abs = game.join("bundle").join("aaaa1111");
    let v = read_w(&wide_path(&abs));
    t.check("CreateFileW absolute redirected", v == b"MOD-A", &v);
    let upper = abs.to_str().unwrap().to_uppercase().replace('\\', "/");
    let v = read_w(&wide(&upper));
    t.check("case and slash insensitive", v == b"MOD-A", &v);
    let v = read_w(&wide(&format!("\\\\?\\{}", abs.display())));
    t.check("\\\\?\\ prefix redirected", v == b"MOD-A", &v);
    let v = read_w(&wide("..\\bundle\\.\\..\\bundle\\aaaa1111"));
    t.check("dot segments resolved", v == b"MOD-A", &v);
    let v = read_a("..\\bundle\\aaaa1111");
    t.check("CreateFileA redirected", v == b"MOD-A", &v);
    let v = read_2(abs.to_str().unwrap());
    t.check("CreateFile2 redirected", v == b"MOD-A", &v);
    let mut info: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileAttributesExW(wide("..\\bundle\\aaaa1111").as_ptr(), GetFileExInfoStandard, &mut info as *mut _ as *mut _) };
    t.check("GetFileAttributesExW reports replacement size", ok != 0 && info.nFileSizeLow == 5, info.nFileSizeLow);
    let mut info_a: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileAttributesExA(c("..\\bundle\\aaaa1111").as_ptr() as *const u8, GetFileExInfoStandard, &mut info_a as *mut _ as *mut _) };
    t.check("GetFileAttributesExA reports replacement size", ok != 0 && info_a.nFileSizeLow == 5, info_a.nFileSizeLow);
    let len = fs::metadata("../bundle/aaaa1111").map(|m| m.len()).unwrap_or(0);
    t.check("std metadata reports replacement size", len == 5, len);
    let mut find: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe { FindFirstFileW(wide("..\\bundle\\aaaa1111").as_ptr(), &mut find) };
    let fname = String::from_utf16_lossy(&find.cFileName[..find.cFileName.iter().position(|&c| c == 0).unwrap_or(0)]);
    t.check("FindFirstFileW reports replacement size under the stock name", h != INVALID_HANDLE_VALUE && find.nFileSizeLow == 5 && fname == "aaaa1111", (&fname, find.nFileSizeLow));
    if h != INVALID_HANDLE_VALUE { unsafe { FindClose(h) }; }
    let mut find: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe { FindFirstFileExW(wide("..\\bundle\\aaaa1111").as_ptr(), 0, &mut find, 0, null(), 0) };
    let fname = String::from_utf16_lossy(&find.cFileName[..find.cFileName.iter().position(|&c| c == 0).unwrap_or(0)]);
    t.check("FindFirstFileExW reports replacement size under the stock name", h != INVALID_HANDLE_VALUE && find.nFileSizeLow == 5 && fname == "aaaa1111", (&fname, find.nFileSizeLow));
    if h != INVALID_HANDLE_VALUE { unsafe { FindClose(h) }; }
    let mut find: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let h = unsafe { FindFirstFileW(wide("..\\bundle\\*").as_ptr(), &mut find) };
    t.check("wildcard find is untouched", h != INVALID_HANDLE_VALUE, ());
    if h != INVALID_HANDLE_VALUE { unsafe { FindClose(h) }; }
    let mut info: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    unsafe { GetFileAttributesExW(wide("..\\bundle\\bbbb2222").as_ptr(), GetFileExInfoStandard, &mut info as *mut _ as *mut _) };
    t.check("unregistered file keeps stock size", info.nFileSizeLow == 7, info.nFileSizeLow);
    let v = read_std_rw("../bundle/aaaa1111");
    t.check("read-write open left on stock", v == b"STOCK-A", &v);
    let v = read_std("settings.ini");
    t.check("files outside bundle untouched", v == b"SETTINGS", &v);

    let short = short_path(&abs);
    if short != abs.to_str().unwrap() {
        let v = read_w(&wide(&short));
        t.check("8.3 short path redirected", v == b"MOD-A", (&short, &v));
    } else {
        println!("skip 8.3 short path (short names disabled on this volume)");
    }
    let r = lib.hash("bundle/aaaa1111");
    t.check("HashFile hashes the stock, not the replacement", r.as_deref() == Ok(sha_a.as_str()), &r);
    let r = lib.hash("mods/alpha/payload/a.bin");
    t.check("HashFile hashes mod files", r.as_deref() == Ok(sha256(b"MOD-A").as_str()), &r);
    let r = lib.hash("binaries/settings.ini");
    t.check("HashFile refuses other folders", r.is_err(), &r);

    let bad = [
        ("wrong sha", "bundle/bbbb2222", "mods/alpha/payload/b.bin", sha_a.clone(), "has changed"),
        ("short sha", "bundle/bbbb2222", "mods/alpha/payload/b.bin", "abc".to_owned(), "64 hex"),
        ("missing stock", "bundle/ffff0000", "mods/alpha/payload/b.bin", sha_b.clone(), "does not exist"),
        ("uppercase stock", "bundle/BBBB2222", "mods/alpha/payload/b.bin", sha_b.clone(), "lowercase"),
        ("stock outside bundle", "binaries/settings.ini", "mods/alpha/payload/b.bin", sha_b.clone(), "under bundle/"),
        ("dotdot replacement", "bundle/bbbb2222", "mods/alpha/../../bundle/aaaa1111", sha_b.clone(), "'..'"),
        ("replacement outside mods", "bundle/bbbb2222", "bundle/aaaa1111", sha_b.clone(), "under mods/"),
        ("backslash replacement", "bundle/bbbb2222", "mods\\alpha\\payload\\b.bin", sha_b.clone(), "forward slashes"),
        ("absolute replacement", "bundle/bbbb2222", "C:/x/b.bin", sha_b.clone(), "under mods/"),
        ("missing replacement", "bundle/bbbb2222", "mods/alpha/payload/none.bin", sha_b.clone(), "missing"),
    ];
    for (name, stock, replacement, sha, expect) in bad {
        let r = lib.add(stock, replacement, &sha);
        t.check(&format!("refuse {name}"), r.as_ref().is_err_and(|e| e.contains(expect)), &r);
    }
    let v = read_std("../bundle/bbbb2222");
    t.check("refused registrations leave stock", v == b"STOCK-B", &v);

    let r = lib.add_new("bundle/cccc3333", "mods/alpha/payload/new.bin");
    t.check("add virtual file", r.is_ok(), &r);
    let v = read_std("../bundle/cccc3333");
    t.check("virtual file served", v == b"NEW", &v);
    let r = lib.add_new("bundle/data/mymod/dddd4444", "mods/alpha/payload/new.bin");
    t.check("add nested virtual file", r.is_ok(), &r);
    let v = read_std("../bundle/data/mymod/dddd4444");
    t.check("nested virtual file served without a real folder", v == b"NEW", &v);
    let attrs = unsafe { GetFileAttributesW(wide("..\\bundle\\data\\mymod\\dddd4444").as_ptr()) };
    t.check("GetFileAttributesW sees a virtual file", attrs != INVALID_FILE_ATTRIBUTES, attrs);
    let attrs = unsafe { GetFileAttributesA(c("..\\bundle\\data\\mymod\\dddd4444").as_ptr() as *const u8) };
    t.check("GetFileAttributesA sees a virtual file", attrs != INVALID_FILE_ATTRIBUTES, attrs);
    let r = lib.add_new("bundle/aaaa1111", "mods/alpha/payload/new.bin");
    t.check("virtual refuses existing stock", r.as_ref().is_err_and(|e| e.contains("exists")), &r);

    let prev = unsafe { (lib.set_enabled)(0) };
    let v = read_std("../bundle/aaaa1111");
    t.check("disabled serves stock", prev == 1 && v == b"STOCK-A", (&prev, &v));
    unsafe { (lib.set_enabled)(1) };
    let v = read_std("../bundle/aaaa1111");
    t.check("re-enabled serves replacement", v == b"MOD-A", &v);

    let held = fs::File::open("../bundle/bbbb2222").unwrap();
    let r = lib.add("bundle/bbbb2222", "mods/alpha/payload/b.bin", &sha_b);
    t.check("file the game holds open is refused", r.as_ref().is_err_and(|e| e.contains("already open")), &r);
    drop(held);
    let r = lib.add("bundle/bbbb2222", "mods/alpha/payload/b.bin", &sha_b);
    t.check("add second file", r.is_ok(), &r);
    fs::remove_file(game.join("mods/alpha/payload/b.bin")).unwrap();
    let v = read_std("../bundle/bbbb2222");
    t.check("vanished replacement falls back to stock", v == b"STOCK-B", &v);

    unsafe { (lib.trace_enable)(1) };
    read_std("../bundle/aaaa1111");
    let dump = lib.sized(lib.trace_dump);
    t.check("trace records serves", dump.contains("serve bundle/aaaa1111 -> mods/alpha/payload/a.bin"), &dump);
    unsafe { (lib.trace_enable)(0) };

    let stats = lib.sized(lib.stats);
    let json: serde_json::Value = serde_json::from_str(&stats).unwrap_or(serde_json::Value::Null);
    let entries = json["entries"].as_array().map(|a| a.len()).unwrap_or(0);
    t.check("stats JSON lists 4 entries", json["installed"] == true && entries == 4, &stats);
    let small = unsafe { (lib.stats)([0 as c_char; 8].as_mut_ptr(), 8) };
    t.check("stats reports needed size", small as usize == stats.len(), small);

    t.check("remove", unsafe { (lib.remove)(c("bundle/aaaa1111").as_ptr()) } == 1, ());
    let v = read_std("../bundle/aaaa1111");
    t.check("removed file serves stock", v == b"STOCK-A", &v);
    t.check("clear returns count", unsafe { (lib.clear)() } == 3, ());
    t.check("opens keep counting", lib.opens("bundle/aaaa1111") >= 10, lib.opens("bundle/aaaa1111"));

    // A second copy from another mod folder must not hook twice.
    let beta = load(game.join("mods/beta/bin/reforge.dll").to_str().unwrap());
    let r = beta.install();
    t.check("second copy refuses to hook", r.as_ref().is_err_and(|e| e.contains("already hooks")), &r);

    println!("\n{} passed, {} failed", t.passed, t.failed);
    std::env::set_current_dir(&base).ok();
    std::process::exit(if t.failed == 0 { 0 } else { 1 });
}
