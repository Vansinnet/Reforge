use std::ffi::{CStr, c_char, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f` and turns a panic into `fallback`, so no panic ever unwinds into
/// the game.
pub fn guarded<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

pub unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// Writes `s` as a NUL-terminated string, truncated to fit `cap` bytes.
pub unsafe fn write_out(out: *mut c_char, cap: c_int, s: &str) {
    if out.is_null() || cap <= 0 {
        return;
    }
    let cap = cap as usize;
    let mut n = s.len().min(cap - 1);
    while n > 0 && !s.is_char_boundary(n) {
        n -= 1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), out as *mut u8, n);
        *out.add(n) = 0;
    }
}

/// snprintf-style: returns the full length of `s` (without NUL). The text is
/// written only when it fits completely, so a caller can retry with
/// `length + 1` bytes.
pub unsafe fn write_sized(out: *mut c_char, cap: c_int, s: &str) -> c_int {
    let need = s.len();
    if !out.is_null() && cap > 0 && (cap as usize) > need {
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), out as *mut u8, need);
            *out.add(need) = 0;
        }
    } else if !out.is_null() && cap > 0 {
        unsafe { *out = 0 };
    }
    need.min(c_int::MAX as usize) as c_int
}
