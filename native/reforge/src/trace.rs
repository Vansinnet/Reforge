use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

const CAP: usize = 4096;

static ON: AtomicBool = AtomicBool::new(false);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);
static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turning tracing on clears the previous buffer. Returns the old state.
pub fn set_enabled(on: bool) -> bool {
    if on {
        LazyLock::force(&START);
        lock().clear();
    }
    ON.swap(on, Ordering::Relaxed)
}

pub fn record(line: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let ms = START.elapsed().as_millis();
    let text = format!("{ms:>9} {}", line());
    let mut lines = lock();
    if lines.len() >= CAP {
        lines.pop_front();
    }
    lines.push_back(text);
}

pub fn dump() -> String {
    let lines = lock();
    let mut out = String::new();
    for line in lines.iter() {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn lock() -> std::sync::MutexGuard<'static, VecDeque<String>> {
    LINES.lock().unwrap_or_else(|e| e.into_inner())
}
