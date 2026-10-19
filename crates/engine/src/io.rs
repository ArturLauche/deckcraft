//! Reading and writing the files a command names by path.
//!
//! Every such read and write goes through [`read_file`] and [`write_file`], so the automation
//! roots ([`crate::file_access`]) are checked in one place. A command module that called
//! `std::fs` itself would bypass them; a test in `file_access` keeps that from happening.

/// The bytes of the file at `path`, inside the automation read root when there is one.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &str) -> Result<Vec<u8>, String> {
    crate::file_access::check_read(path)?;
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

#[cfg(target_arch = "wasm32")]
pub fn read_file(path: &str) -> Result<Vec<u8>, String> {
    Err(format!("{path}: files are opened through the browser on the web"))
}

/// Write `bytes` to the file at `path`, inside the automation write root when there is one.
///
/// The bytes go to a temporary sibling first and are renamed over `path`, so a failed save never
/// truncates the old file. The sibling is in the same folder, so it is inside the root whenever
/// `path` is.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    crate::file_access::check_write(path)?;
    let err = |e: std::io::Error| format!("{path}: {e}");
    // A uniquely named temporary sibling, created exclusively (never following or reusing an
    // existing file or symlink), is renamed over the destination: concurrent saves never share
    // temporary bytes.
    let mut attempts = 0;
    let (tmp, mut file) = loop {
        let tmp = format!("{path}.{}.{}.saving", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempts < 16 => attempts += 1,
            Err(e) => return Err(err(e)),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err(e));
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn write_file(path: &str, _bytes: &[u8]) -> Result<(), String> {
    Err(format!("{path}: saving to a path is not available in the browser"))
}

/// Like [`read_file`], but a missing file is `Ok(None)` (a sidecar that has never been written).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file_optional(path: &str) -> Result<Option<Vec<u8>>, String> {
    crate::file_access::check_read(path)?;
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn read_file_optional(path: &str) -> Result<Option<Vec<u8>>, String> {
    Err(format!("{path}: files are opened through the browser on the web"))
}

/// Whether two paths name the same file. The file system resolves `.`, `..` and symlinks; a
/// lexical fallback keeps a new path (which cannot be canonicalised yet) comparable. Kept here
/// because this module is the one place engine code may touch the file system directly.
pub fn paths_equal(a: &str, b: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return a == b;
    }
    fn lexical(path: &str) -> std::path::PathBuf {
        use std::path::Component;
        let mut out = std::path::PathBuf::new();
        for part in std::path::Path::new(path).components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }
    lexical(a) == lexical(b)
}

/// Check that `path` may be written (the automation write root), without touching it. Video
/// export hands the checked path to ffmpeg, which writes the file itself.
#[cfg(not(target_arch = "wasm32"))]
pub fn check_write_path(path: &str) -> Result<(), String> {
    crate::file_access::check_write(path)
}

/// Read a file in one of the application's own folders (user templates, settings): unlike
/// [`read_file`], no automation root applies, because the path never comes from a command.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_app_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Like [`read_app_file`], but a missing file is `Ok(None)` rather than an error: the usual case
/// for settings that have never been written.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_app_file_optional(path: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The browser has no settings folder: sync reports this rather than pretending to save.
#[cfg(target_arch = "wasm32")]
pub fn read_app_file_optional(path: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    Err(format!("{}: the browser has no settings folder", path.display()))
}

/// The browser has no settings folder: sync reports this rather than pretending to save.
#[cfg(target_arch = "wasm32")]
pub fn write_app_file(path: &std::path::Path, _bytes: &[u8]) -> Result<(), String> {
    Err(format!("{}: the browser has no settings folder", path.display()))
}

/// The browser has no settings folder: sync reports this rather than pretending to save.
#[cfg(target_arch = "wasm32")]
pub fn write_app_private_file(path: &std::path::Path, _bytes: &[u8]) -> Result<(), String> {
    Err(format!("{}: the browser has no settings folder", path.display()))
}

/// No file permissions in the browser.
#[cfg(target_arch = "wasm32")]
pub fn restrict_to_owner(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

/// The browser has no temporary files.
#[cfg(target_arch = "wasm32")]
pub fn create_temp(prefix: &str) -> Result<(std::path::PathBuf, std::fs::File), String> {
    Err(format!("{prefix}: the browser has no temporary files"))
}

/// Nothing to remove in the browser.
#[cfg(target_arch = "wasm32")]
pub fn remove_file(_path: &std::path::Path) {}

/// Make a file readable and writable only by its owner (Unix; a no-op elsewhere). Settings sync
/// stores credentials in its config, so the file must not be world-readable.
#[cfg(not(target_arch = "wasm32"))]
pub fn restrict_to_owner(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("{}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Write a file in one of the application's own folders, atomically (a temporary sibling renamed
/// over it). Like [`read_app_file`], this is for app-owned paths only.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_app_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    write_app_file_inner(path, bytes, false)
}

/// Like [`write_app_file`], but the temporary file is owner-only from the moment it is created:
/// settings sync writes credentials here, and they must never be world-readable, even briefly.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_app_private_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    write_app_file_inner(path, bytes, true)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_app_file_inner(path: &std::path::Path, bytes: &[u8], private: bool) -> Result<(), String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut attempts = 0;
    let (tmp, mut file) = loop {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
        let tmp = path.with_file_name(format!(".{name}.{}.{}.saving", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        // A private file is created with its final mode, so there is no moment when it is
        // world-readable (unlike opening and then chmodding it).
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&tmp) {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempts < 16 => attempts += 1,
            Err(e) => return Err(err(e)),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err(e));
    }
    Ok(())
}

/// The regular files directly in an application folder, sorted by name. A missing folder is an
/// empty list, not an error.
#[cfg(not(target_arch = "wasm32"))]
pub fn app_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<std::path::PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))).collect();
    out.sort();
    out
}

/// A fresh temporary file for an export's own scratch data (never a path a command named), with
/// its name. The caller removes it.
#[cfg(not(target_arch = "wasm32"))]
pub fn create_temp(prefix: &str) -> Result<(std::path::PathBuf, std::fs::File), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    for _ in 0..32 {
        let path = dir.join(format!("{prefix}-{}-{}.tmp", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err(format!("{}: no free temporary name", dir.display()))
}

/// Remove a file this engine wrote (a scratch file, or a partial export).
#[cfg(not(target_arch = "wasm32"))]
pub fn remove_file(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// The size in bytes of the file at `path`; `None` when it can't be read (or is outside the
/// automation read root, so File › Info doesn't reveal files the agent may not read).
#[cfg(not(target_arch = "wasm32"))]
pub fn file_len(path: &str) -> Option<u64> {
    crate::file_access::check_read(path).ok()?;
    std::fs::metadata(path).ok().map(|m| m.len())
}

#[cfg(target_arch = "wasm32")]
pub fn file_len(_path: &str) -> Option<u64> {
    None
}
