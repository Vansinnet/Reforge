use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};

use sha2::{Digest, Sha256};

use crate::paths::{bypass, check_rel, normalize, to_wide_nul};


pub struct Root {
    /// Normalized (see `paths::normalize`) game folder, in its long and its
    /// 8.3 short form, so either spelling of the game path matches.
    pub norms: Vec<String>,
    pub native: PathBuf,
}

impl Root {
    pub fn relative<'a>(&self, norm: &'a str) -> Option<&'a str> {
        self.norms.iter().find_map(|root| crate::paths::relative_to(norm, root))
    }
}

pub struct Entry {
    pub replacement_rel: String,
    /// Absolute replacement path, NUL-terminated UTF-16, ready for CreateFileW.
    pub replacement_wide: Vec<u16>,
    pub is_virtual: bool,
    pub stock_sha256: Option<String>,
    pub served: AtomicU32,
}

pub static ROOT: OnceLock<Root> = OnceLock::new();
pub static INSTALLED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(true);
pub static ENTRIES: LazyLock<RwLock<HashMap<String, Entry>>> = LazyLock::new(|| RwLock::new(HashMap::new()));
/// Read-only opens under `bundle/` since install, redirected or not.
static OPENS: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn set_root(native: PathBuf) -> Result<&'static Root, String> {
    if let Some(root) = ROOT.get() {
        return Ok(root);
    }
    let mut norms: Vec<String> = Vec::new();
    for form in crate::paths::spellings(&native) {
        if let Some(norm) = normalize(&form) {
            let norm = norm.trim_end_matches('/').to_owned();
            if !norms.contains(&norm) {
                norms.push(norm);
            }
        }
    }
    if norms.is_empty() {
        return Err("cannot resolve the game folder path".to_owned());
    }
    let _ = ROOT.set(Root { norms, native });
    Ok(ROOT.get().expect("root was just set"))
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) -> bool {
    ENABLED.swap(on, Ordering::Relaxed)
}

pub fn note_open(rel: &str) {
    let mut opens = OPENS.lock().unwrap_or_else(|e| e.into_inner());
    *opens.entry(rel.to_owned()).or_insert(0) += 1;
}

pub fn opens(rel: &str) -> u32 {
    OPENS.lock().unwrap_or_else(|e| e.into_inner()).get(rel).copied().unwrap_or(0)
}

fn root() -> Result<&'static Root, String> {
    ROOT.get().ok_or_else(|| "call Reforge_Install first".to_owned())
}

fn native_path(root: &Root, rel: &str) -> PathBuf {
    let mut path = root.native.clone();
    for part in rel.split('/') {
        path.push(part);
    }
    path
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    bypass(|| {
        let mut file = std::fs::File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let digest = hasher.finalize();
        let mut hex = String::with_capacity(64);
        for byte in digest.iter() {
            hex.push_str(&format!("{byte:02x}"));
        }
        Ok(hex)
    })
}

pub fn hash_rel(rel: &str) -> Result<String, String> {
    let root = root()?;
    if rel.starts_with("bundle/") {
        check_rel(rel, "bundle/", false, "path")?;
    } else {
        check_rel(rel, "mods/", false, "path")?;
    }
    hash_file(&native_path(root, rel))
}

fn is_file(path: &Path) -> bool {
    bypass(|| path.is_file())
}

pub fn add(stock: &str, replacement: &str, sha256: Option<&str>, is_virtual: bool) -> Result<(), String> {
    let root = root()?;
    check_rel(stock, "bundle/", true, "stock path")?;
    check_rel(replacement, "mods/", false, "replacement")?;

    let stock_path = native_path(root, stock);
    let stock_exists = is_file(&stock_path);
    let mut expected = None;

    if is_virtual {
        if stock_exists {
            return Err(format!(
                "{stock} exists in the game; register it with Reforge_Add and its sha256 instead"
            ));
        }
    } else {
        let want = sha256.unwrap_or("").to_ascii_lowercase();
        if want.len() != 64 || !want.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("sha256 must be 64 hex digits".to_owned());
        }
        if !stock_exists {
            return Err(format!("{stock} does not exist in this game version"));
        }
        let actual = hash_file(&stock_path)?;
        if actual != want {
            return Err(format!("{stock} has changed (sha256 {actual}, expected {want})"));
        }
        if in_use(&stock_path) {
            return Err(format!(
                "{stock} is already open in the game (it loaded before Reforge started), so it stays stock"
            ));
        }
        expected = Some(want);
    }

    let replacement_path = native_path(root, replacement);
    if !is_file(&replacement_path) {
        return Err(format!("replacement {replacement} is missing"));
    }

    let entry = Entry {
        replacement_rel: replacement.to_owned(),
        replacement_wide: to_wide_nul(&replacement_path),
        is_virtual,
        stock_sha256: expected,
        served: AtomicU32::new(0),
    };
    ENTRIES.write().unwrap_or_else(|e| e.into_inner()).insert(stock.to_owned(), entry);
    crate::trace::record(|| format!("add {stock} -> {replacement}"));
    Ok(())
}

pub fn remove(stock: &str) -> bool {
    let removed = ENTRIES.write().unwrap_or_else(|e| e.into_inner()).remove(stock).is_some();
    if removed {
        crate::trace::record(|| format!("remove {stock}"));
    }
    removed
}

pub fn clear() -> usize {
    let mut entries = ENTRIES.write().unwrap_or_else(|e| e.into_inner());
    let n = entries.len();
    entries.clear();
    n
}

/// The replacement path for `rel` without counting a serve (metadata queries).
pub fn peek(rel: &str) -> Option<Vec<u16>> {
    if !enabled() {
        return None;
    }
    let entries = ENTRIES.read().unwrap_or_else(|e| e.into_inner());
    entries.get(rel).map(|entry| entry.replacement_wide.clone())
}

/// True when some handle in this process (the game's) already has the file
/// open. Serving a replacement for a file the game is already reading would
/// mix two files' bytes, so such files are refused.
fn in_use(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    bypass(|| match std::fs::OpenOptions::new().read(true).share_mode(0).open(path) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    })
}

/// The replacement for `rel`, if one is registered and redirects are on.
pub fn lookup(rel: &str) -> Option<(Vec<u16>, String)> {
    if !enabled() {
        return None;
    }
    let entries = ENTRIES.read().unwrap_or_else(|e| e.into_inner());
    let entry = entries.get(rel)?;
    entry.served.fetch_add(1, Ordering::Relaxed);
    Some((entry.replacement_wide.clone(), entry.replacement_rel.clone()))
}

pub fn stats_json() -> String {
    let entries = ENTRIES.read().unwrap_or_else(|e| e.into_inner());
    let mut keys: Vec<&String> = entries.keys().collect();
    keys.sort();
    let list: Vec<serde_json::Value> = keys
        .into_iter()
        .map(|key| {
            let entry = &entries[key];
            serde_json::json!({
                "key": key,
                "replacement": entry.replacement_rel,
                "virtual": entry.is_virtual,
                "sha256": entry.stock_sha256,
                "served": entry.served.load(Ordering::Relaxed),
                "opens": opens(key),
            })
        })
        .collect();
    let bundle_files_opened = OPENS.lock().unwrap_or_else(|e| e.into_inner()).len();
    serde_json::json!({
        "abi": crate::ABI_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "installed": INSTALLED.load(Ordering::Relaxed),
        "enabled": enabled(),
        "root": ROOT.get().map(|r| r.native.display().to_string()),
        "bundle_files_opened": bundle_files_opened,
        "entries": list,
    })
    .to_string()
}
