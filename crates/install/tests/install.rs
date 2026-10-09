//! Each installer against temp folders: what lands where, what is refused, what uninstall removes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use artcraft_toolbox_install::{AppInfo, Error, Layout, Limits, install, uninstall};
use artcraft_toolbox_release::PackageKind;
use zip::write::SimpleFileOptions;

fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("artcraft-toolbox-install-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn layout(root: &Path) -> Layout {
    Layout {
        apps: root.join("apps"),
        desktop_entries: Some(root.join("share/applications")),
        icons: Some(root.join("share/icons")),
        start_menu: None,
        downloads: root.join("downloads"),
    }
}

const ICON: &[u8] = b"\x89PNG fake icon";

fn app() -> AppInfo<'static> {
    AppInfo { id: "fakecraft", name: "FakeCraft", bundle_id: "ai.storyteller.fakecraft", tagline: "Testing things", version: "1.2.0", icon_png: Some(ICON) }
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        if name.ends_with('/') {
            w.add_directory(*name, SimpleFileOptions::default()).unwrap();
        } else {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
    }
    w.finish().unwrap().into_inner()
}

fn write(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = root.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// What the crafts publish: one folder with the exes, licences and `portable.txt`.
fn portable_zip() -> Vec<u8> {
    zip_of(&[
        ("fakecraft-1.2.0-windows-x64-portable/", b""),
        ("fakecraft-1.2.0-windows-x64-portable/fakecraft.exe", b"MZ app"),
        ("fakecraft-1.2.0-windows-x64-portable/fakecraft-cli.exe", b"MZ cli"),
        ("fakecraft-1.2.0-windows-x64-portable/portable.txt", b"portable mode"),
        ("fakecraft-1.2.0-windows-x64-portable/licenses/LICENSE-MIT", b"MIT"),
    ])
}

#[test]
fn a_portable_zip_installs_without_its_portable_marker() {
    let root = temp("zip");
    let l = layout(&root);
    let pkg = write(&root, "pkg.zip", &portable_zip());
    let exe = install(&l, &app(), PackageKind::PortableZip, &pkg).unwrap();
    assert_eq!(exe, root.join("apps/FakeCraft/1.2.0/fakecraft.exe"));
    assert_eq!(std::fs::read(&exe).unwrap(), b"MZ app");
    assert!(exe.with_file_name("fakecraft-cli.exe").is_file());
    assert!(exe.with_file_name("licenses").join("LICENSE-MIT").is_file());
    assert!(!exe.with_file_name("portable.txt").exists(), "portable mode would keep app data in the version folder");
    // No staging leftovers beside it.
    assert_eq!(std::fs::read_dir(root.join("apps/FakeCraft")).unwrap().count(), 1);
    // Never over an existing installation.
    assert!(matches!(install(&l, &app(), PackageKind::PortableZip, &pkg), Err(Error::Exists(_))));
    uninstall(&l, &app(), PackageKind::PortableZip, &exe).unwrap();
    assert!(!root.join("apps/FakeCraft").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn hostile_zips_are_refused_and_leave_nothing_behind() {
    let cases: [(&str, Vec<u8>); 5] = [
        ("escape", zip_of(&[("../evil.exe", b"x"), ("fakecraft.exe", b"MZ")])),
        ("absolute", zip_of(&[("/etc/evil", b"x"), ("fakecraft.exe", b"MZ")])),
        ("no exe", zip_of(&[("app/other.exe", b"MZ"), ("app/readme.txt", b"hi")])),
        ("not a zip", b"PK but not really".to_vec()),
        ("symlink", {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            w.start_file("fakecraft.exe", SimpleFileOptions::default()).unwrap();
            w.write_all(b"MZ").unwrap();
            w.add_symlink("link", "/etc/passwd", SimpleFileOptions::default()).unwrap();
            w.finish().unwrap().into_inner()
        }),
    ];
    for (what, bytes) in cases {
        let root = temp("hostile");
        let pkg = write(&root, "pkg.zip", &bytes);
        let r = install(&layout(&root), &app(), PackageKind::PortableZip, &pkg);
        assert!(matches!(r, Err(Error::BadPackage(_))), "{what}: {r:?}");
        let left: Vec<_> =
            std::fs::read_dir(root.join("apps/FakeCraft")).map(|d| d.filter_map(Result::ok).map(|e| e.file_name()).collect()).unwrap_or_default();
        assert!(left.is_empty(), "{what}: left {left:?}");
        assert!(!root.join("evil.exe").exists() && !root.join("apps/evil.exe").exists(), "{what}");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn zip_bombs_and_entry_floods_hit_the_limits() {
    let root = temp("limits");
    let pkg = write(&root, "pkg.zip", &zip_of(&[("fakecraft.exe", &[0u8; 4096]), ("b.bin", &[0u8; 4096])]));
    let small = Limits { max_entries: 10, max_total_bytes: 5000 };
    let err = portable_extract(&pkg, &root.join("out1"), small).unwrap_err();
    assert!(err.to_string().contains("size limit"), "{err}");
    let few = Limits { max_entries: 1, max_total_bytes: 1 << 20 };
    let err = portable_extract(&pkg, &root.join("out2"), few).unwrap_err();
    assert!(err.to_string().contains("too many entries"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

fn portable_extract(pkg: &Path, dest: &Path, limits: Limits) -> Result<(), Error> {
    artcraft_toolbox_install::extract_zip(pkg, dest, limits)
}

fn fake_appimage() -> Vec<u8> {
    let mut b = b"\x7fELF\x02\x01\x01\x00AI\x02\x00\x00\x00\x00\x00".to_vec();
    b.extend_from_slice(&[0u8; 256]);
    b
}

#[test]
fn an_appimage_installs_with_its_desktop_entry_and_icon() {
    let root = temp("appimage");
    let l = layout(&root);
    let pkg = write(&root, "pkg.AppImage", &fake_appimage());
    let path = install(&l, &app(), PackageKind::AppImage, &pkg).unwrap();
    assert_eq!(path, root.join("apps/fakecraft/1.2.0/fakecraft.AppImage"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
    }
    let entry = std::fs::read_to_string(root.join("share/applications/ai.storyteller.fakecraft.desktop")).unwrap();
    assert!(entry.contains("Name=FakeCraft\n") && entry.contains("Icon=ai.storyteller.fakecraft\n"), "{entry}");
    assert!(entry.contains(&format!("Exec=\"{}\" %F\n", path.display())), "{entry}");
    assert_eq!(std::fs::read(root.join("share/icons/hicolor/128x128/apps/ai.storyteller.fakecraft.png")).unwrap(), ICON);
    assert!(matches!(install(&l, &app(), PackageKind::AppImage, &pkg), Err(Error::Exists(_))));

    uninstall(&l, &app(), PackageKind::AppImage, &path).unwrap();
    assert!(!root.join("apps/fakecraft").exists());
    assert!(!root.join("share/applications/ai.storyteller.fakecraft.desktop").exists());
    assert!(!root.join("share/icons/hicolor/128x128/apps/ai.storyteller.fakecraft.png").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn someone_elses_desktop_entry_is_left_alone() {
    let root = temp("foreign");
    let l = layout(&root);
    std::fs::create_dir_all(root.join("share/applications")).unwrap();
    let theirs = "[Desktop Entry]\nName=FakeCraft (system)\n";
    std::fs::write(root.join("share/applications/ai.storyteller.fakecraft.desktop"), theirs).unwrap();
    let pkg = write(&root, "pkg.AppImage", &fake_appimage());
    let path = install(&l, &app(), PackageKind::AppImage, &pkg).unwrap();
    uninstall(&l, &app(), PackageKind::AppImage, &path).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("share/applications/ai.storyteller.fakecraft.desktop")).unwrap(), theirs);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn not_an_appimage() {
    let root = temp("notappimage");
    for bytes in [b"#!/bin/sh\necho hi".to_vec(), b"\x7fELF\x02\x01\x01\x00\x00\x00\x00".to_vec(), Vec::new()] {
        let pkg = write(&root, "pkg.AppImage", &bytes);
        assert!(matches!(install(&layout(&root), &app(), PackageKind::AppImage, &pkg), Err(Error::BadPackage(_))));
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn uninstall_refuses_paths_that_are_not_installations() {
    let root = temp("refuse");
    let l = layout(&root);
    for (kind, path) in [
        (PackageKind::AppImage, root.clone()),
        (PackageKind::AppImage, root.join("apps/fakecraft/1.2.0/other.AppImage")),
        (PackageKind::AppImage, root.join("apps/othercraft/1.2.0/fakecraft.AppImage")),
        (PackageKind::PortableZip, root.join("apps/FakeCraft/9.9.9/fakecraft.exe")),
        (PackageKind::PortableZip, PathBuf::from("/")),
    ] {
        assert!(matches!(uninstall(&l, &app(), kind, &path), Err(Error::Refused(_))), "{kind:?} {}", path.display());
    }
    assert!(root.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unsupported_kinds() {
    let root = temp("kinds");
    let pkg = write(&root, "pkg", b"x");
    for kind in [PackageKind::Msi, PackageKind::Deb, PackageKind::TarGz] {
        assert!(matches!(install(&layout(&root), &app(), kind, &pkg), Err(Error::Unsupported(_))));
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A real disk image with a real (empty) app bundle, through hdiutil.
#[cfg(target_os = "macos")]
#[test]
fn a_dmg_installs_its_app_and_is_detached() {
    use std::process::Command;
    let root = temp("dmg");
    let src = root.join("src");
    let make_bundle = |id: &str| {
        let contents = src.join("FakeCraft.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(contents.join("MacOS/fakecraft"), b"#!/bin/sh\n").unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string></dict></plist>\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink("/Applications", src.join("Applications")).ok();
    };
    let make_dmg = |name: &str| {
        let dmg = root.join(name);
        let ok = Command::new("hdiutil")
            .args(["create", "-quiet", "-volname", "FakeCraft", "-fs", "HFS+", "-format", "UDZO", "-srcfolder"])
            .arg(&src)
            .arg(&dmg)
            .status()
            .unwrap();
        assert!(ok.success());
        dmg
    };
    make_bundle("ai.storyteller.fakecraft");
    let dmg = make_dmg("good.dmg");
    let l = layout(&root);
    let app_path = install(&l, &app(), PackageKind::Dmg, &dmg).unwrap();
    assert_eq!(app_path, root.join("apps/FakeCraft.app"));
    assert!(app_path.join("Contents/MacOS/fakecraft").is_file());
    // The image was detached and its mount point removed.
    assert_eq!(std::fs::read_dir(&l.downloads).unwrap().count(), 0);
    assert!(matches!(install(&l, &app(), PackageKind::Dmg, &dmg), Err(Error::Exists(_))));
    uninstall(&l, &app(), PackageKind::Dmg, &app_path).unwrap();
    assert!(!app_path.exists());

    // A disk image holding another app is refused.
    std::fs::remove_dir_all(&src).unwrap();
    make_bundle("com.example.other");
    let wrong = make_dmg("wrong.dmg");
    let r = install(&l, &app(), PackageKind::Dmg, &wrong);
    assert!(matches!(r, Err(Error::BadPackage(ref m)) if m.contains("com.example.other")), "{r:?}");
    assert!(!root.join("apps/FakeCraft.app").exists());
    let _ = std::fs::remove_dir_all(&root);
}
