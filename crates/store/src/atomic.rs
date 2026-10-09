//! Crash-safe file replacement, ported from PhotoCraft's `format::atomic` (simplified: no
//! failure-injection seam).
//!
//! [`atomic_write`] never truncates the destination. It writes a temporary file in the **same
//! directory**, flushes it (`sync_all`), renames it over the destination and, on Unix, fsyncs the
//! directory. If anything fails, the destination keeps its previous bytes and the temporary file
//! is removed. On Windows a rename refused with `PermissionDenied` (an antivirus scanner or the
//! search indexer holding the file) is retried with backoff.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Atomically replace (or create) `path` with `bytes`, creating its folder if needed.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: not a file path", path.display())))?;
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(temp_name(&name.to_string_lossy()));
    if let Err(e) = write_new(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = rename_with_retry(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // The new bytes are in place; a failed directory fsync can't be undone, so it is best effort.
    let _ = sync_dir(&dir);
    Ok(())
}

fn write_new(tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let (mut left, mut delay) = if cfg!(windows) { (7u32, 10u64) } else { (0, 0) };
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && left > 0 => {
                left -= 1;
                std::thread::sleep(std::time::Duration::from_millis(delay));
                delay = delay.saturating_mul(2);
            }
            r => return r,
        }
    }
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// A unique hidden temporary name beside `file`.
fn temp_name(file: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem: String = file.chars().take(64).collect();
    format!(".{stem}.{}-{n}.tmp", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;

    #[test]
    fn replaces_creates_and_leaves_no_temp_files() {
        let dir = temp("atomic");
        let path = dir.join("nested").join("settings.json");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<String> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["settings.json"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_cleans_up() {
        let dir = temp("atomic-fail");
        std::fs::create_dir_all(&dir).unwrap();
        // A directory where the file should go: the rename fails, the directory survives.
        let target = dir.join("inventory.json");
        std::fs::create_dir_all(target.join("child")).unwrap();
        assert!(atomic_write(&target, b"new").is_err());
        assert!(target.join("child").is_dir());
        let leftovers = std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".tmp"))).count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
