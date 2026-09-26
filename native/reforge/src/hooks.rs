use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, Ordering};

use minhook::MinHook;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::CREATEFILE2_EXTENDED_PARAMETERS;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcessId};

use crate::paths::{Guard, find_game_root, normalize, wide_len};
use crate::state::{self, INSTALLED, ROOT};
use crate::trace;

type CreateFileWFn =
    unsafe extern "system" fn(*const u16, u32, u32, *const SECURITY_ATTRIBUTES, u32, u32, HANDLE) -> HANDLE;
type CreateFileAFn =
    unsafe extern "system" fn(*const u8, u32, u32, *const SECURITY_ATTRIBUTES, u32, u32, HANDLE) -> HANDLE;
type CreateFile2Fn =
    unsafe extern "system" fn(*const u16, u32, u32, u32, *const CREATEFILE2_EXTENDED_PARAMETERS) -> HANDLE;

static ORIG_W: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static ORIG_A: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static ORIG_2: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_APPEND_DATA: u32 = 0x0004;
const FILE_WRITE_EA: u32 = 0x0010;
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
const DELETE: u32 = 0x0001_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const WRITE_MASK: u32 = GENERIC_WRITE
    | GENERIC_ALL
    | FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | DELETE
    | WRITE_DAC
    | WRITE_OWNER;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

fn read_only(access: u32, disposition: u32, flags: u32) -> bool {
    access & WRITE_MASK == 0
        && (disposition == OPEN_EXISTING || disposition == OPEN_ALWAYS)
        && flags & FILE_FLAG_DELETE_ON_CLOSE == 0
}

/// Decides whether an open of `path` should be served from a replacement.
/// Must be called with a `Guard` held.
fn decide(path: &[u16], access: u32, disposition: u32, flags: u32) -> Option<Vec<u16>> {
    let root = ROOT.get()?;
    let norm = normalize(path)?;
    let rel = root.relative(&norm)?;
    if !rel.starts_with("bundle/") {
        return None;
    }
    if !read_only(access, disposition, flags) {
        trace::record(|| format!("skip {rel} (not a read-only open)"));
        return None;
    }
    state::note_open(rel);
    match state::lookup(rel) {
        Some((wide, replacement)) => {
            trace::record(|| format!("serve {rel} -> {replacement}"));
            Some(wide)
        }
        None => {
            trace::record(|| format!("stock {rel}"));
            None
        }
    }
}

fn decide_ptr(path: *const u16, access: u32, disposition: u32, flags: u32) -> Option<Vec<u16>> {
    catch_unwind(AssertUnwindSafe(|| {
        let len = unsafe { wide_len(path) }?;
        let slice = unsafe { std::slice::from_raw_parts(path, len) };
        decide(slice, access, disposition, flags)
    }))
    .ok()
    .flatten()
}

fn ansi_to_wide(path: *const u8) -> Option<Vec<u16>> {
    if path.is_null() {
        return None;
    }
    let len = unsafe { std::ffi::CStr::from_ptr(path as *const _) }.to_bytes().len();
    if len == 0 || len > 32767 {
        return None;
    }
    let n = unsafe { MultiByteToWideChar(CP_ACP, 0, path, len as i32, null_mut(), 0) };
    if n <= 0 {
        return None;
    }
    let mut wide = vec![0u16; n as usize + 1];
    let written = unsafe { MultiByteToWideChar(CP_ACP, 0, path, len as i32, wide.as_mut_ptr(), n) };
    if written != n {
        return None;
    }
    Some(wide)
}

fn fallback_note(replacement: &[u16]) {
    let code = unsafe { GetLastError() };
    trace::record(|| {
        let text = String::from_utf16_lossy(&replacement[..replacement.len().saturating_sub(1)]);
        format!("replacement open failed (error {code}), serving stock: {text}")
    });
}

unsafe extern "system" fn detour_w(
    name: *const u16,
    access: u32,
    share: u32,
    sa: *const SECURITY_ATTRIBUTES,
    disposition: u32,
    flags: u32,
    template: HANDLE,
) -> HANDLE {
    let orig: CreateFileWFn = unsafe { std::mem::transmute(ORIG_W.load(Ordering::Acquire)) };
    let Some(_guard) = Guard::enter() else {
        return unsafe { orig(name, access, share, sa, disposition, flags, template) };
    };
    if let Some(replacement) = decide_ptr(name, access, disposition, flags) {
        let handle = unsafe { orig(replacement.as_ptr(), access, share, sa, disposition, flags, template) };
        if handle != INVALID_HANDLE_VALUE {
            return handle;
        }
        fallback_note(&replacement);
    }
    unsafe { orig(name, access, share, sa, disposition, flags, template) }
}

unsafe extern "system" fn detour_a(
    name: *const u8,
    access: u32,
    share: u32,
    sa: *const SECURITY_ATTRIBUTES,
    disposition: u32,
    flags: u32,
    template: HANDLE,
) -> HANDLE {
    let orig: CreateFileAFn = unsafe { std::mem::transmute(ORIG_A.load(Ordering::Acquire)) };
    let Some(_guard) = Guard::enter() else {
        return unsafe { orig(name, access, share, sa, disposition, flags, template) };
    };
    let replacement = catch_unwind(AssertUnwindSafe(|| {
        let wide = ansi_to_wide(name)?;
        decide(&wide[..wide.len() - 1], access, disposition, flags)
    }))
    .ok()
    .flatten();
    if let Some(replacement) = replacement {
        let orig_w: CreateFileWFn = unsafe { std::mem::transmute(ORIG_W.load(Ordering::Acquire)) };
        let handle = unsafe { orig_w(replacement.as_ptr(), access, share, sa, disposition, flags, template) };
        if handle != INVALID_HANDLE_VALUE {
            return handle;
        }
        fallback_note(&replacement);
    }
    unsafe { orig(name, access, share, sa, disposition, flags, template) }
}

unsafe extern "system" fn detour_2(
    name: *const u16,
    access: u32,
    share: u32,
    disposition: u32,
    params: *const CREATEFILE2_EXTENDED_PARAMETERS,
) -> HANDLE {
    let orig: CreateFile2Fn = unsafe { std::mem::transmute(ORIG_2.load(Ordering::Acquire)) };
    let Some(_guard) = Guard::enter() else {
        return unsafe { orig(name, access, share, disposition, params) };
    };
    let flags = if params.is_null() { 0 } else { unsafe { (*params).dwFileFlags } };
    if let Some(replacement) = decide_ptr(name, access, disposition, flags) {
        let handle = unsafe { orig(replacement.as_ptr(), access, share, disposition, params) };
        if handle != INVALID_HANDLE_VALUE {
            return handle;
        }
        fallback_note(&replacement);
    }
    unsafe { orig(name, access, share, disposition, params) }
}

fn hook(proc_name: &str, detour: *mut c_void, slot: &AtomicPtr<c_void>) -> Result<*mut c_void, String> {
    let mut last = String::new();
    for module in ["kernelbase.dll", "kernel32.dll"] {
        match unsafe { MinHook::create_hook_api(module, proc_name, detour) } {
            Ok(original) => {
                slot.store(original, Ordering::Release);
                return Ok(original);
            }
            Err(status) => last = format!("{module}!{proc_name}: {status:?}"),
        }
    }
    Err(format!("cannot hook {last}"))
}

/// Finds the game folder, claims the process-wide hook slot and installs the
/// three hooks. Safe to call repeatedly.
pub fn install() -> Result<(), String> {
    let _lock = INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }

    let root_path = find_game_root()?;
    state::set_root(root_path)?;

    // Another copy of reforge.dll (from a different mod folder) may already
    // hook this process. Two sets of hooks would fight, so refuse.
    let name: Vec<u16> = format!("Local\\Reforge.Hooks.{}", unsafe { GetCurrentProcessId() })
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mutex = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    if mutex.is_null() {
        return Err(format!("cannot create the Reforge process marker (error {})", unsafe { GetLastError() }));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(mutex) };
        return Err(
            "another copy of reforge.dll already hooks this game; use the loaded copy (GetModuleHandleA(\"reforge.dll\"))"
                .to_owned(),
        );
    }

    let result = (|| {
        for (proc_name, detour, slot) in [
            ("CreateFileW", detour_w as *mut c_void, &ORIG_W),
            ("CreateFileA", detour_a as *mut c_void, &ORIG_A),
            ("CreateFile2", detour_2 as *mut c_void, &ORIG_2),
        ] {
            hook(proc_name, detour, slot)?;
        }
        unsafe { MinHook::enable_all_hooks() }.map_err(|status| format!("cannot enable hooks: {status:?}"))
    })();

    if let Err(message) = result {
        // Nothing was enabled; leave the process as we found it.
        unsafe { CloseHandle(mutex) };
        return Err(message);
    }

    // The marker handle is intentionally kept open for the process lifetime.
    INSTALLED.store(true, Ordering::Release);
    trace::record(|| "installed".to_owned());
    Ok(())
}
