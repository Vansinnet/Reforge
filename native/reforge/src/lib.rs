//! Reforge: lets Darktide mods replace the game's own bundle files without
//! touching the installed game.
//!
//! A mod loads this library through LuaJIT's FFI and registers pairs of
//! `bundle/<file>` (a stock game file, pinned by SHA-256) and
//! `mods/<Mod>/<file>` (the replacement). After `Reforge_Install`, read-only
//! opens of a registered stock file are served from the replacement.
//!
//! Scope, on purpose:
//! - Hooks `CreateFileW`, `CreateFileA` and `CreateFile2` in this process only.
//! - Only read-only opens under `<game>/bundle/` are ever redirected.
//! - Replacements must live under `<game>/mods/`.
//! - No network access, no file writes, no other memory patching.
//!
//! The C ABI is documented in `docs/ABI.md`.
//!
//! # Safety (all exports)
//!
//! String arguments must be NULL or NUL-terminated UTF-8. Output buffers must be
//! NULL or point to at least `cap` writable bytes. Every export catches panics.
#![allow(clippy::missing_safety_doc)]

mod ffi_util;
mod hooks;
mod paths;
mod state;
mod trace;

use std::ffi::{c_char, c_int};

use ffi_util::{cstr, guarded, write_out, write_sized};

/// Bumped only for incompatible ABI changes. New exports do not bump it.
pub const ABI_VERSION: c_int = 1;

#[unsafe(no_mangle)]
pub extern "C" fn Reforge_Abi() -> c_int {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_Version(out: *mut c_char, cap: c_int) -> c_int {
    guarded(0, || {
        unsafe { write_out(out, cap, env!("CARGO_PKG_VERSION")) };
        1
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_Install(err: *mut c_char, errlen: c_int) -> c_int {
    guarded(0, || match hooks::install() {
        Ok(()) => 1,
        Err(message) => {
            unsafe { write_out(err, errlen, &message) };
            0
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_Add(
    stock_rel: *const c_char,
    replacement_rel: *const c_char,
    expect_stock_sha256: *const c_char,
    err: *mut c_char,
    errlen: c_int,
) -> c_int {
    guarded(0, || {
        let result = match unsafe { (cstr(stock_rel), cstr(replacement_rel), cstr(expect_stock_sha256)) } {
            (Some(stock), Some(replacement), Some(sha)) => state::add(stock, replacement, Some(sha), false),
            _ => Err("stock, replacement and sha256 must be non-null UTF-8 strings".to_owned()),
        };
        report(result, err, errlen)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_AddNew(
    virtual_rel: *const c_char,
    replacement_rel: *const c_char,
    err: *mut c_char,
    errlen: c_int,
) -> c_int {
    guarded(0, || {
        let result = match unsafe { (cstr(virtual_rel), cstr(replacement_rel)) } {
            (Some(stock), Some(replacement)) => state::add(stock, replacement, None, true),
            _ => Err("path and replacement must be non-null UTF-8 strings".to_owned()),
        };
        report(result, err, errlen)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_Remove(stock_rel: *const c_char) -> c_int {
    guarded(0, || match unsafe { cstr(stock_rel) } {
        Some(stock) => state::remove(stock) as c_int,
        None => 0,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn Reforge_Clear() -> c_int {
    guarded(0, || state::clear().min(c_int::MAX as usize) as c_int)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_Opens(stock_rel: *const c_char) -> c_int {
    guarded(-1, || match unsafe { cstr(stock_rel) } {
        Some(stock) => state::opens(stock).min(c_int::MAX as u32) as c_int,
        None => -1,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_HashFile(rel: *const c_char, out: *mut c_char, cap: c_int) -> c_int {
    guarded(0, || {
        let result = match unsafe { cstr(rel) } {
            Some(rel) => state::hash_rel(rel),
            None => Err("path must be a non-null UTF-8 string".to_owned()),
        };
        match result {
            Ok(hex) => {
                unsafe { write_out(out, cap, &hex) };
                1
            }
            Err(message) => {
                unsafe { write_out(out, cap, &message) };
                0
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn Reforge_SetEnabled(on: c_int) -> c_int {
    guarded(0, || state::set_enabled(on != 0) as c_int)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_Stats(out: *mut c_char, cap: c_int) -> c_int {
    guarded(-1, || unsafe { write_sized(out, cap, &state::stats_json()) })
}

#[unsafe(no_mangle)]
pub extern "C" fn Reforge_TraceEnable(on: c_int) -> c_int {
    guarded(0, || trace::set_enabled(on != 0) as c_int)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Reforge_TraceDump(out: *mut c_char, cap: c_int) -> c_int {
    guarded(-1, || unsafe { write_sized(out, cap, &trace::dump()) })
}

fn report(result: Result<(), String>, err: *mut c_char, errlen: c_int) -> c_int {
    match result {
        Ok(()) => 1,
        Err(message) => {
            unsafe { write_out(err, errlen, &message) };
            0
        }
    }
}
