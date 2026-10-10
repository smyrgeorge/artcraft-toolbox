//! The toolbox replacing itself (docs/architecture.md § 7): where this process's own
//! installation is ([`locate`]), whether it can be replaced ([`writable`]), the swap of a staged
//! version into its place ([`swap`]), and starting the new one ([`relaunch`]).
//!
//! The staged version is placed by the ordinary installers ([`crate::place`] into the toolbox's
//! own data folder) and verified like any craft; what is special is the swap: the running
//! program is the one being replaced, so nothing here refuses a running app. The old version is
//! kept beside the new one for a manual recovery: in `previous_dir` on macOS and Linux, and as
//! `<name>.previous` beside the exe on Windows, where a running executable can be renamed but
//! not deleted or overwritten (and only within its own volume).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use artcraft_toolbox_release::PackageKind;

use crate::{Error, Result, io, occupied};

/// The toolbox's executables, as the portable zip names them.
pub const EXE: &str = "artcraft-toolbox.exe";
pub const CLI_EXE: &str = "artcraft-toolbox-cli.exe";
/// Set by the AppImage runtime to the AppImage's own path.
pub const APPIMAGE_ENV: &str = "APPIMAGE";

/// Where the toolbox this process runs from is installed, and as what: the `.app` bundle on
/// macOS (the executable sits in `<X>.app/Contents/MacOS/`), the AppImage on Linux (its runtime
/// sets `APPIMAGE`), or `artcraft-toolbox.exe` beside this executable on Windows. `None` for
/// anything else: a `cargo run` build, a package-manager install under `/usr/bin`, a CLI
/// unzipped on its own.
pub fn locate(exe: &Path, env: impl Fn(&str) -> Option<OsString>) -> Option<(PackageKind, PathBuf)> {
    if let Some(appimage) = env(APPIMAGE_ENV).filter(|p| !p.is_empty()).map(PathBuf::from)
        && appimage.is_file()
    {
        return Some((PackageKind::AppImage, appimage));
    }
    let mut up = exe.ancestors();
    let (_, macos, contents, bundle) = (up.next()?, up.next()?, up.next()?, up.next()?);
    if macos.file_name().is_some_and(|n| n == "MacOS")
        && contents.file_name().is_some_and(|n| n == "Contents")
        && bundle.extension().is_some_and(|x| x == "app")
    {
        return Some((PackageKind::Dmg, bundle.to_path_buf()));
    }
    let name = exe.file_name()?.to_str()?;
    if name.eq_ignore_ascii_case(EXE) || name.eq_ignore_ascii_case(CLI_EXE) {
        let main = exe.with_file_name(EXE);
        if main.is_file() {
            return Some((PackageKind::PortableZip, main));
        }
    }
    None
}

/// The program to start for an installation of `kind` at `path`: the bundle's
/// `CFBundleExecutable`, else `path` itself.
pub fn executable(kind: PackageKind, path: &Path) -> Result<PathBuf> {
    match kind {
        PackageKind::Dmg => {
            let name = crate::dmg::bundle_executable(path).ok_or_else(|| Error::BadPackage(format!("{} names no executable", path.display())))?;
            Ok(path.join("Contents").join("MacOS").join(name))
        }
        PackageKind::PortableZip | PackageKind::AppImage => Ok(path.to_path_buf()),
        other => Err(Error::Unsupported(other)),
    }
}

/// Can the installation at `path` be replaced? Its folder must take a new file (a probe is
/// written and removed): `/Applications` for an admin user yes, `/usr/bin` or `Program Files`
/// without elevation no. The toolbox never elevates (AGENTS.md).
pub fn writable(path: &Path) -> Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).ok_or_else(|| Error::Refused(format!("{} has no folder", path.display())))?;
    let probe = crate::staging(dir, "write-probe");
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(Error::Refused(format!("{} can't be changed by this user ({e}); update the toolbox the way it was installed", dir.display()))),
    }
}

/// Where things are after a [`swap`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swapped {
    /// The installation, now the staged version (the same path as before).
    pub active: PathBuf,
    /// Where the old version went.
    pub previous: PathBuf,
}

/// Where the old version of `kind` at `current` goes: `previous_dir/<name>`, or `.previous`
/// beside a Windows exe (see the module docs).
fn previous_of(kind: PackageKind, current: &Path, previous_dir: &Path) -> Result<PathBuf> {
    let name = current.file_name().ok_or_else(|| Error::Refused(format!("{} has no name", current.display())))?;
    Ok(match kind {
        PackageKind::PortableZip => current.with_file_name(format!("{}.previous", name.to_string_lossy())),
        _ => previous_dir.join(name),
    })
}

/// Put the staged version (`staged`, as [`crate::place`] returned it) where `current` is, keeping
/// the old one (`previous_dir`, see the module docs). On failure the old version is moved back.
/// Nothing checks whether `current` is running: it usually is.
pub fn swap(kind: PackageKind, current: &Path, staged: &Path, previous_dir: &Path) -> Result<Swapped> {
    if !occupied(staged) {
        return Err(Error::Refused(format!("the downloaded version is no longer at {}", staged.display())));
    }
    if current.file_name() != staged.file_name() {
        return Err(Error::Refused(format!("{} can't replace {} (different names)", staged.display(), current.display())));
    }
    let previous = previous_of(kind, current, previous_dir)?;
    if let Some(dir) = previous.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir.display(), &e))?;
    }
    remove(&previous)?;
    move_path(current, &previous)?;
    let restore = || {
        let _ = remove(current);
        let _ = move_path(&previous, current);
    };
    if let Err(e) = move_path(staged, current) {
        restore();
        return Err(e);
    }
    if kind == PackageKind::PortableZip {
        // The CLI exe travels with the app exe: beside the staged one, and beside the current.
        let cli_new = staged.with_file_name(CLI_EXE);
        let cli = current.with_file_name(CLI_EXE);
        if cli_new.is_file() {
            let cli_previous = cli.with_file_name(format!("{CLI_EXE}.previous"));
            let _ = remove(&cli_previous);
            if occupied(&cli)
                && let Err(e) = move_path(&cli, &cli_previous)
            {
                log::warn!("{}: kept as it was: {e}", cli.display());
            } else if let Err(e) = move_path(&cli_new, &cli) {
                log::warn!("{}: not updated: {e}", cli.display());
                let _ = move_path(&cli_previous, &cli);
            }
        }
    }
    #[cfg(unix)]
    if kind == PackageKind::AppImage {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(current, std::fs::Permissions::from_mode(0o755));
    }
    // The staged version's own (now empty) folders in the data folder.
    if let Some(dir) = staged.parent() {
        let _ = std::fs::remove_dir(dir);
        if let Some(parent) = dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    Ok(Swapped { active: current.to_path_buf(), previous })
}

/// Remove what an earlier [`swap`] left beside a Windows exe (`<name>.previous`), once the old
/// process is gone. Best effort: a file still in use stays for the next start.
pub fn clean_previous(kind: PackageKind, current: &Path) {
    if kind != PackageKind::PortableZip {
        return;
    }
    for name in [EXE, CLI_EXE] {
        let leftover = current.with_file_name(format!("{name}.previous"));
        if leftover.is_file() {
            let _ = std::fs::remove_file(&leftover);
        }
    }
}

/// Start `exe` with `args` as a new process, detached from this one (macOS: through Launch
/// Services, as a new instance of the bundle), so the caller can exit and the new version
/// carries on.
pub fn relaunch(kind: PackageKind, path: &Path, args: &[OsString]) -> Result<()> {
    let exe = executable(kind, path)?;
    if !exe.exists() {
        return Err(Error::Refused(format!("{} is missing", exe.display())));
    }
    let mut cmd = match kind {
        PackageKind::Dmg => {
            let mut c = Command::new("open");
            c.arg("-n").arg(path);
            if !args.is_empty() {
                c.arg("--args").args(args);
            }
            c
        }
        _ => {
            let mut c = Command::new(&exe);
            c.args(args);
            if let Some(dir) = exe.parent() {
                c.current_dir(dir);
            }
            crate::process::detach(&mut c);
            c
        }
    };
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| io(exe.display(), &e))?;
    // `open` returns at once; a direct child is reaped by its own thread so it never lingers as
    // a zombie of a parent that is about to exit anyway.
    let _ = std::thread::Builder::new().name("relaunch".into()).spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

fn remove(path: &Path) -> Result<()> {
    if !occupied(path) {
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| io(path.display(), &e))?;
    if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }.map_err(|e| io(path.display(), &e))
}

/// Rename, or copy and delete when `to` is on another volume. A Windows exe in use can't be
/// deleted: the copy stays and the error names it.
fn move_path(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            if std::fs::symlink_metadata(from).map_err(|e| io(from.display(), &e))?.is_dir() {
                copy_tree(from, to)?;
                std::fs::remove_dir_all(from).map_err(|e| io(from.display(), &e))
            } else {
                std::fs::copy(from, to).map_err(|e| io(to.display(), &e))?;
                std::fs::remove_file(from).map_err(|e| io(from.display(), &e))
            }
        }
        Err(e) => Err(io(to.display(), &e)),
    }
}

/// A bundle to another volume: `ditto` on macOS (keeps the code signature's extended attributes
/// and permissions), a plain recursive copy elsewhere.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        return crate::dmg::run(Command::new("ditto").arg(from).arg(to), "ditto");
    }
    std::fs::create_dir_all(to).map_err(|e| io(to.display(), &e))?;
    for entry in std::fs::read_dir(from).map_err(|e| io(from.display(), &e))? {
        let entry = entry.map_err(|e| io(from.display(), &e))?;
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| io(from.display(), &e))?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| io(target.display(), &e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("artcraft-toolbox-selfupdate-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn locates_a_bundle_an_appimage_and_a_portable_exe() {
        let root = temp("locate");
        let bundle = root.join("ArtCraft Toolbox.app");
        let exe = bundle.join("Contents/MacOS/ArtCraft Toolbox");
        assert_eq!(locate(&exe, no_env), Some((PackageKind::Dmg, bundle.clone())));
        // A Linux AppImage: the runtime says where the image is; the exe is the mounted binary.
        let appimage = root.join("artcraft-toolbox.AppImage");
        std::fs::write(&appimage, b"x").unwrap();
        let env = |k: &str| (k == APPIMAGE_ENV).then(|| appimage.clone().into_os_string());
        assert_eq!(locate(Path::new("/tmp/.mount_x/usr/bin/artcraft-toolbox"), env), Some((PackageKind::AppImage, appimage.clone())));
        // An APPIMAGE that doesn't exist is ignored.
        let gone = |k: &str| (k == APPIMAGE_ENV).then(|| OsString::from("/nope/x.AppImage"));
        assert_eq!(locate(Path::new("/tmp/.mount_x/usr/bin/artcraft-toolbox"), gone), None);
        // Windows: the app exe beside whichever of the two is running.
        let dir = root.join("portable");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(EXE), b"x").unwrap();
        assert_eq!(locate(&dir.join(EXE), no_env), Some((PackageKind::PortableZip, dir.join(EXE))));
        assert_eq!(locate(&dir.join(CLI_EXE), no_env), Some((PackageKind::PortableZip, dir.join(EXE))));
        assert_eq!(locate(&dir.join("Artcraft-Toolbox.EXE"), no_env), Some((PackageKind::PortableZip, dir.join(EXE))));
        // Not an installation: a cargo build, a CLI on its own, /usr/bin.
        for other in ["target/debug/artcraft-toolbox", "/usr/bin/artcraft-toolbox", "/x/artcraft-toolbox-cli", "/x/other.exe"] {
            assert_eq!(locate(Path::new(other), no_env), None, "{other}");
        }
        assert_eq!(locate(&root.join("lonely").join(CLI_EXE), no_env), None, "no app exe beside it");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn writable_probes_the_folder() {
        let root = temp("writable");
        let file = root.join("artcraft-toolbox.AppImage");
        std::fs::write(&file, b"x").unwrap();
        writable(&file).unwrap();
        assert!(std::fs::read_dir(&root).unwrap().count() == 1, "the probe is removed");
        assert!(writable(Path::new("artcraft-toolbox.AppImage")).is_err(), "no folder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = root.join("locked");
            std::fs::create_dir(&locked).unwrap();
            std::fs::write(locked.join("x"), b"x").unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
            // root can write anywhere; everyone else gets a refusal naming the folder.
            if let Err(e) = writable(&locked.join("x")) {
                assert!(e.to_string().contains("locked") && e.to_string().contains("way it was installed"), "{e}");
            }
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn swaps_a_file_and_keeps_the_previous_one() {
        let root = temp("swap-file");
        let current = root.join("apps").join("artcraft-toolbox.AppImage");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, b"old").unwrap();
        let staged = root.join("data/self-update/artcraft-toolbox/0.2.0/artcraft-toolbox.AppImage");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, b"new").unwrap();
        let previous_dir = root.join("data/self-update/previous");
        let s = swap(PackageKind::AppImage, &current, &staged, &previous_dir).unwrap();
        assert_eq!(s.active, current);
        assert_eq!(s.previous, previous_dir.join("artcraft-toolbox.AppImage"));
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert_eq!(std::fs::read(&s.previous).unwrap(), b"old");
        assert!(!staged.exists() && !staged.parent().unwrap().exists(), "the staged version's folders are gone");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&current).unwrap().permissions().mode() & 0o111, 0o111);
        }
        // A second swap replaces the previous one.
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, b"newer").unwrap();
        swap(PackageKind::AppImage, &current, &staged, &previous_dir).unwrap();
        assert_eq!(std::fs::read(&s.previous).unwrap(), b"new");
        // A staged version that is gone, or has another name, is refused and changes nothing.
        assert!(swap(PackageKind::AppImage, &current, &staged, &previous_dir).unwrap_err().to_string().contains("no longer"));
        std::fs::write(root.join("other.AppImage"), b"x").unwrap();
        assert!(swap(PackageKind::AppImage, &current, &root.join("other.AppImage"), &previous_dir).unwrap_err().to_string().contains("different names"));
        assert_eq!(std::fs::read(&current).unwrap(), b"newer");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn swaps_a_bundle_and_restores_it_when_the_new_one_cant_move() {
        let root = temp("swap-bundle");
        let current = root.join("Applications").join("ArtCraft Toolbox.app");
        std::fs::create_dir_all(current.join("Contents/MacOS")).unwrap();
        std::fs::write(current.join("Contents/MacOS/ArtCraft Toolbox"), b"old").unwrap();
        let staged = root.join("data/self-update/artcraft-toolbox/0.2.0/ArtCraft Toolbox.app");
        std::fs::create_dir_all(staged.join("Contents/MacOS")).unwrap();
        std::fs::write(staged.join("Contents/MacOS/ArtCraft Toolbox"), b"new").unwrap();
        let previous_dir = root.join("data/self-update/previous");
        let s = swap(PackageKind::Dmg, &current, &staged, &previous_dir).unwrap();
        assert_eq!(std::fs::read(current.join("Contents/MacOS/ArtCraft Toolbox")).unwrap(), b"new");
        assert_eq!(std::fs::read(s.previous.join("Contents/MacOS/ArtCraft Toolbox")).unwrap(), b"old");
        assert_eq!(executable(PackageKind::AppImage, Path::new("/x/a.AppImage")).unwrap(), PathBuf::from("/x/a.AppImage"));
        // The staged bundle vanishes between the check and the move: the old one is back.
        let gone = root.join("data/self-update/artcraft-toolbox/0.3.0/ArtCraft Toolbox.app");
        std::fs::create_dir_all(&gone).unwrap();
        // A directory where a file is expected under it makes the rename fail on every OS.
        let blocker = current.join("Contents/MacOS/ArtCraft Toolbox");
        let before = std::fs::read(&blocker).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(swap(PackageKind::Dmg, &current, &gone, &previous_dir).is_err());
        assert_eq!(std::fs::read(&blocker).unwrap(), before, "untouched");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn swaps_a_portable_exe_with_its_cli_beside_it() {
        let root = temp("swap-exe");
        let dir = root.join("ArtCraft Toolbox");
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join(EXE);
        std::fs::write(&current, b"old").unwrap();
        std::fs::write(dir.join(CLI_EXE), b"old-cli").unwrap();
        let staged_dir = root.join("data/self-update/ArtCraft Toolbox/0.2.0");
        std::fs::create_dir_all(&staged_dir).unwrap();
        std::fs::write(staged_dir.join(EXE), b"new").unwrap();
        std::fs::write(staged_dir.join(CLI_EXE), b"new-cli").unwrap();
        let s = swap(PackageKind::PortableZip, &current, &staged_dir.join(EXE), &root.join("unused")).unwrap();
        assert_eq!(s.previous, dir.join("artcraft-toolbox.exe.previous"));
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join(CLI_EXE)).unwrap(), b"new-cli");
        assert_eq!(std::fs::read(&s.previous).unwrap(), b"old");
        assert_eq!(std::fs::read(dir.join("artcraft-toolbox-cli.exe.previous")).unwrap(), b"old-cli");
        assert!(!root.join("unused").exists(), "Windows keeps the previous exe beside, in its volume");
        clean_previous(PackageKind::PortableZip, &current);
        assert!(!s.previous.exists() && !dir.join("artcraft-toolbox-cli.exe.previous").exists());
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn relaunching_a_missing_program_is_an_error() {
        assert!(relaunch(PackageKind::AppImage, Path::new("/definitely/not/here.AppImage"), &[]).is_err());
        assert!(matches!(executable(PackageKind::Msi, Path::new("/x")), Err(Error::Unsupported(_))));
    }
}
