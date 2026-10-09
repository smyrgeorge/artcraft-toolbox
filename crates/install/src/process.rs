//! Opening an installed app, and telling whether one is running.

use std::path::Path;
use std::process::{Command, Stdio};

use artcraft_toolbox_release::PackageKind;

use crate::{Error, Result, io};

/// Start the app at `path` (as [`crate::install`] returned it), detached from the toolbox.
pub fn launch(kind: PackageKind, path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(Error::Refused(format!("{} is missing; reinstall the app", path.display())));
    }
    let mut cmd = match kind {
        PackageKind::Dmg => {
            let mut c = Command::new("open");
            c.arg(path);
            c
        }
        PackageKind::PortableZip | PackageKind::AppImage => {
            let mut c = Command::new(path);
            if let Some(dir) = path.parent() {
                c.current_dir(dir);
            }
            detach(&mut c);
            c
        }
        other => return Err(Error::Unsupported(other)),
    };
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| io(path.display(), &e))?;
    // Reap it when it exits, so it never lingers as a zombie of the toolbox.
    let _ = std::thread::Builder::new().name("reap".into()).spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(windows)]
fn detach(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
fn detach(_c: &mut Command) {}

/// Is a process running from `path`? Best effort (`pgrep -f` on macOS and Linux; on Windows the
/// running executable is locked, so removing it fails by itself).
pub fn is_running(path: &Path) -> bool {
    if cfg!(windows) {
        return false;
    }
    let Some(p) = path.to_str() else { return false };
    // pgrep takes a regular expression: escape the path.
    let pattern: String = p.chars().flat_map(|c| if "\\.+*?()[]{}^$|".contains(c) { vec!['\\', c] } else { vec![c] }).collect();
    Command::new("pgrep").args(["-f", "--", &pattern]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launching_a_missing_app_is_an_error() {
        let missing = Path::new("/definitely/not/here/x.AppImage");
        assert!(matches!(launch(PackageKind::AppImage, missing), Err(Error::Refused(_))));
        assert!(!is_running(missing));
    }

    #[cfg(unix)]
    #[test]
    fn a_running_process_is_found_by_its_path() {
        // `sleep` launched by its full path stands in for an app.
        let sleep = ["/bin/sleep", "/usr/bin/sleep"].into_iter().map(Path::new).find(|p| p.exists());
        let Some(sleep) = sleep else { return };
        let mut child = Command::new(sleep).arg("5").spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(is_running(sleep));
        let _ = child.kill();
        let _ = child.wait();
    }
}
