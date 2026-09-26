use std::cell::Cell;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::Storage::FileSystem::{GetFullPathNameW, GetLongPathNameW, GetShortPathNameW};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};

thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Marks the current thread as inside Reforge. File opens made while a guard
/// is held (by the hooks themselves, or by Reforge's own hashing) are passed
/// straight to the original functions.
pub struct Guard(());

impl Guard {
    pub fn enter() -> Option<Guard> {
        BUSY.try_with(|busy| {
            if busy.get() {
                None
            } else {
                busy.set(true);
                Some(Guard(()))
            }
        })
        .ok()
        .flatten()
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = BUSY.try_with(|busy| busy.set(false));
    }
}

/// Runs `f` with hooks bypassed on this thread.
pub fn bypass<R>(f: impl FnOnce() -> R) -> R {
    let guard = Guard::enter();
    let result = f();
    drop(guard);
    result
}

/// Length of a NUL-terminated UTF-16 string, capped at 32767 units.
pub unsafe fn wide_len(p: *const u16) -> Option<usize> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while n < 32767 {
        if unsafe { *p.add(n) } == 0 {
            return Some(n);
        }
        n += 1;
    }
    None
}

pub fn to_wide_nul(path: &Path) -> Vec<u16> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    wide
}

/// Canonical comparison form: absolute, no `\\?\` prefix, forward slashes,
/// lowercase. Returns `None` when Windows cannot resolve the path.
pub fn normalize(wide: &[u16]) -> Option<String> {
    let mut input: Vec<u16> = wide.to_vec();
    input.push(0);
    let mut buf: Vec<u16> = vec![0; 520];
    loop {
        let n = unsafe { GetFullPathNameW(input.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), null_mut()) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            buf.truncate(n);
            break;
        }
        if n > 32768 {
            return None;
        }
        buf.resize(n + 1, 0);
    }
    let mut text = String::from_utf16_lossy(&buf).replace('\\', "/");
    for prefix in ["//?/unc/", "//?/UNC/"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = format!("//{rest}");
        }
    }
    for prefix in ["//?/", "//./", "/??/"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.to_owned();
        }
    }
    while text.contains("//") && !text.starts_with("//") {
        text = text.replace("//", "/");
    }
    Some(text.to_lowercase())
}

/// The path as given plus its long and 8.3 short spellings (without NUL).
pub fn spellings(path: &Path) -> Vec<Vec<u16>> {
    let input = to_wide_nul(path);
    let mut out = vec![input[..input.len() - 1].to_vec()];
    for convert in [GetLongPathNameW, GetShortPathNameW] {
        let mut buf: Vec<u16> = vec![0; 1024];
        let n = bypass(|| unsafe { convert(input.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) }) as usize;
        if n > 0 && n < buf.len() {
            buf.truncate(n);
            out.push(buf);
        }
    }
    out
}

/// `norm` relative to `root`, both in normalized form.
pub fn relative_to<'a>(norm: &'a str, root: &str) -> Option<&'a str> {
    let rest = norm.strip_prefix(root)?;
    let rest = rest.strip_prefix('/')?;
    if rest.is_empty() { None } else { Some(rest) }
}

/// Validates a game-relative path given by a mod: forward slashes, no `..`,
/// no drive letters, and inside `prefix`.
pub fn check_rel(rel: &str, prefix: &str, lowercase: bool, what: &str) -> Result<(), String> {
    let bad = rel.is_empty()
        || rel.len() > 1024
        || !rel.starts_with(prefix)
        || rel.ends_with('/')
        || rel.contains('\\')
        || rel.contains(':')
        || rel.contains("//")
        || rel.chars().any(|c| c.is_control())
        || rel.split('/').any(|seg| seg == "." || seg == "..");
    if bad {
        return Err(format!(
            "{what} must be a path under {prefix} with forward slashes and no '..' (got {rel:?})"
        ));
    }
    if lowercase && rel != rel.to_lowercase() {
        return Err(format!("{what} must be lowercase (got {rel:?})"));
    }
    Ok(())
}

fn own_module_path() -> Result<PathBuf, String> {
    let mut module: HMODULE = null_mut();
    let anchor = own_module_path as *const u16;
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            anchor,
            &mut module,
        )
    };
    if ok == 0 {
        return Err("cannot determine this DLL's own path".to_owned());
    }
    let mut buf: Vec<u16> = vec![0; 520];
    loop {
        let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            return Err("cannot determine this DLL's own path".to_owned());
        }
        if n < buf.len() - 1 {
            buf.truncate(n);
            return Ok(PathBuf::from(OsString::from_wide(&buf)));
        }
        if buf.len() > 32768 {
            return Err("this DLL's path is too long".to_owned());
        }
        buf.resize(buf.len() * 2, 0);
    }
}

/// The game folder is the nearest ancestor of this DLL that contains both
/// `bundle` and `binaries`.
pub fn find_game_root() -> Result<PathBuf, String> {
    let dll = own_module_path()?;
    bypass(|| {
        for dir in dll.ancestors().skip(1) {
            if dir.join("bundle").is_dir() && dir.join("binaries").is_dir() {
                return Ok(dir.to_path_buf());
            }
        }
        Err(format!(
            "no folder above {} contains both bundle and binaries; reforge.dll must be inside the game's mods folder",
            dll.display()
        ))
    })
}
