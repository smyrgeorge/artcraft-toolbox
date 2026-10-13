//! Opening an installed app, and telling whether one is running.
//!
//! An app the toolbox opens finds [`MANAGED_ENV`]`=1` in its environment: the toolbox keeps it up
//! to date, so a craft with its own update check can leave that to the toolbox
//! (docs/release-contract.md). The toolbox's own `ARTCRAFT_TOOLBOX_*` variables (a GitHub token
//! among them) never reach the app.

use std::path::Path;
use std::process::{Command, Stdio};

use artcraft_toolbox_release::PackageKind;

use crate::{Error, Result, io};

/// Set to `1` for the apps the toolbox opens.
pub const MANAGED_ENV: &str = "ARTCRAFT_TOOLBOX_MANAGED";

/// Start the app at `path` (as [`crate::install`] returned it), detached from the toolbox, with
/// `args` (its control channel's, for an agent to drive it). On macOS, Launch Services hands
/// them only to a copy it starts: an app already running is just brought forward.
pub fn launch(kind: PackageKind, path: &Path, args: &[String]) -> Result<()> {
    if !path.exists() {
        return Err(Error::Refused(format!("{} is missing; reinstall the app", path.display())));
    }
    let mut cmd = command(kind, path, args)?;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| io(path.display(), &e))?;
    // Reap it when it exits, so it never lingers as a zombie of the toolbox.
    let _ = std::thread::Builder::new().name("reap".into()).spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// The command that starts the app at `path` with `args`.
fn command(kind: PackageKind, path: &Path, args: &[String]) -> Result<Command> {
    Ok(match kind {
        PackageKind::Dmg => {
            // Launch Services starts the app with a fresh environment; `--env` adds to it.
            let mut c = Command::new("open");
            c.arg("--env").arg(format!("{MANAGED_ENV}=1")).arg(path);
            if !args.is_empty() {
                c.arg("--args").args(args);
            }
            c
        }
        PackageKind::PortableZip | PackageKind::AppImage => {
            let mut c = Command::new(path);
            c.args(args);
            if let Some(dir) = path.parent() {
                c.current_dir(dir);
            }
            for (key, _) in std::env::vars_os() {
                if key.to_str().is_some_and(|k| k.starts_with("ARTCRAFT_TOOLBOX_")) {
                    c.env_remove(&key);
                }
            }
            c.env(MANAGED_ENV, "1");
            detach(&mut c);
            c
        }
        other => return Err(Error::Unsupported(other)),
    })
}

/// Does something accept connections on `127.0.0.1:<port>`? Asked for at most a quarter second:
/// whether an app's control channel is open.
pub fn port_answers(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(250)).is_ok()
}

#[cfg(windows)]
pub(crate) fn detach(c: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(windows))]
pub(crate) fn detach(_c: &mut Command) {}

/// Is a process running from `path` (a file, or a folder such as an `.app` bundle)? Best effort:
/// `pgrep -f` on macOS and Linux, the processes' image paths through PowerShell on Windows.
pub fn is_running(path: &Path) -> bool {
    if cfg!(windows) {
        // The path travels in an environment variable, so no quoting can break the command.
        let script = "if (Get-Process | Where-Object { $_.Path -and ($_.Path -eq $env:TOOLBOX_PATH -or $_.Path.StartsWith($env:TOOLBOX_PATH + '\\', 'OrdinalIgnoreCase')) }) { exit 0 } else { exit 1 }";
        return crate::powershell(script)
            .env("TOOLBOX_PATH", path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
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
        assert!(matches!(launch(PackageKind::AppImage, missing, &[]), Err(Error::Refused(_))));
        assert!(!is_running(missing));
    }

    #[test]
    fn the_arguments_reach_the_app() {
        let args = ["--control".to_string(), "7878".to_string()];
        let list = |c: &Command| c.get_args().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        let app = Path::new("/Applications/PhotoCraft.app");
        // Through Launch Services: after `--args`, the last of `open`'s own.
        assert_eq!(
            list(&command(PackageKind::Dmg, app, &args).unwrap()),
            ["--env", "ARTCRAFT_TOOLBOX_MANAGED=1", "/Applications/PhotoCraft.app", "--args", "--control", "7878"]
        );
        assert_eq!(list(&command(PackageKind::Dmg, app, &[]).unwrap()), ["--env", "ARTCRAFT_TOOLBOX_MANAGED=1", "/Applications/PhotoCraft.app"]);
        let exe = Path::new("/apps/photocraft/0.5.0/photocraft.AppImage");
        assert_eq!(list(&command(PackageKind::AppImage, exe, &args).unwrap()), ["--control", "7878"]);
        assert!(matches!(command(PackageKind::Msi, exe, &args), Err(Error::Unsupported(_))));
    }

    #[test]
    fn a_listening_port_answers() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // Only this way round: once it's closed, another process may take the port at once.
        assert!(port_answers(port));
    }

    #[cfg(windows)]
    #[test]
    fn a_running_process_is_found_by_its_path() {
        let ping = Path::new(r"C:\Windows\System32\PING.EXE");
        let mut child = Command::new(ping).args(["-n", "6", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(is_running(ping));
        assert!(is_running(Path::new(r"C:\Windows\System32")), "a folder counts when a process runs from inside it");
        let _ = child.kill();
        let _ = child.wait();
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
