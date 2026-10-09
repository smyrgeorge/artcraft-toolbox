//! Each installer against temp folders: what lands where, what is refused, what uninstall removes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use artcraft_toolbox_install::{AppInfo, Current, Error, Layout, Limits, activate, find_unmanaged, install, place, remove_version, uninstall};
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
        kept: root.join("kept"),
    }
}

fn at(version: &'static str) -> AppInfo<'static> {
    AppInfo { version, ..app() }
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
    // The exact quoting is unit-tested (`exec_quote`); a Windows path's backslashes get escaped.
    assert!(entry.contains("Exec=\"") && entry.contains("fakecraft.AppImage\" %F\n"), "{entry}");
    #[cfg(unix)]
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

#[test]
fn appimage_versions_side_by_side_update_roll_back_and_prune() {
    let root = temp("appimage-versions");
    let l = layout(&root);
    let pkg = write(&root, "pkg.AppImage", &fake_appimage());
    let entry = root.join("share/applications/ai.storyteller.fakecraft.desktop");
    let v1 = install(&l, &at("1.2.0"), PackageKind::AppImage, &pkg).unwrap();
    // The update goes in beside it; nothing changes until it is activated.
    let v2 = place(&l, &at("1.3.0"), PackageKind::AppImage, &pkg).unwrap();
    assert!(std::fs::read_to_string(&entry).unwrap().contains("X-ArtCraft-Toolbox-Version=1.2.0\n"));
    let a = activate(&l, &at("1.3.0"), PackageKind::AppImage, &v2, Some(Current { path: &v1, version: "1.2.0" })).unwrap();
    assert_eq!((a.active.as_path(), a.previous), (v2.as_path(), None));
    assert!(std::fs::read_to_string(&entry).unwrap().contains("X-ArtCraft-Toolbox-Version=1.3.0\n"));
    assert!(v1.is_file(), "the previous version is kept");
    // Both are found on disk; rolling back is activating the old one again.
    let mut found: Vec<String> = find_unmanaged(&l, &app(), PackageKind::AppImage).into_iter().map(|f| f.version).collect();
    found.sort();
    assert_eq!(found, ["1.2.0", "1.3.0"]);
    activate(&l, &at("1.2.0"), PackageKind::AppImage, &v1, Some(Current { path: &v2, version: "1.3.0" })).unwrap();
    assert!(std::fs::read_to_string(&entry).unwrap().contains("X-ArtCraft-Toolbox-Version=1.2.0\n"));
    // Pruning removes a version's folder, never the entry of the active one.
    remove_version(&l, &at("1.3.0"), PackageKind::AppImage, &v2).unwrap();
    assert!(!v2.exists() && v1.is_file() && entry.is_file());
    assert!(matches!(remove_version(&l, &at("1.3.0"), PackageKind::AppImage, &v1), Err(Error::Refused(_))), "the path must match the version");
    uninstall(&l, &at("1.2.0"), PackageKind::AppImage, &v1).unwrap();
    assert!(!entry.exists() && !root.join("apps/fakecraft").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn portable_versions_side_by_side() {
    let root = temp("zip-versions");
    let l = layout(&root);
    let pkg = write(&root, "pkg.zip", &portable_zip());
    let v1 = install(&l, &at("1.2.0"), PackageKind::PortableZip, &pkg).unwrap();
    let v2 = place(&l, &at("1.3.0"), PackageKind::PortableZip, &pkg).unwrap();
    assert_eq!(v2, root.join("apps/FakeCraft/1.3.0/fakecraft.exe"));
    activate(&l, &at("1.3.0"), PackageKind::PortableZip, &v2, Some(Current { path: &v1, version: "1.2.0" })).unwrap();
    assert_eq!(find_unmanaged(&l, &app(), PackageKind::PortableZip).len(), 2);
    remove_version(&l, &at("1.2.0"), PackageKind::PortableZip, &v1).unwrap();
    assert!(!v1.exists() && v2.is_file());
    uninstall(&l, &at("1.3.0"), PackageKind::PortableZip, &v2).unwrap();
    assert!(!root.join("apps/FakeCraft").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn nothing_is_found_where_nothing_was_installed() {
    let root = temp("none");
    for kind in [PackageKind::AppImage, PackageKind::PortableZip, PackageKind::Dmg, PackageKind::Msi] {
        assert!(find_unmanaged(&layout(&root), &app(), kind).is_empty());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Real disk images with real (empty, unsigned) app bundles, through hdiutil: install, update,
/// roll back, find, prune, uninstall.
#[cfg(target_os = "macos")]
#[test]
fn dmg_versions_swap_through_the_kept_folder() {
    use artcraft_toolbox_install::{bundle_version, verify_signature};
    use artcraft_toolbox_model::Trust;
    use std::process::Command;
    let root = temp("dmg");
    let src = root.join("src");
    let make_dmg = |name: &str, id: &str, version: &str| {
        let _ = std::fs::remove_dir_all(&src);
        let contents = src.join("FakeCraft.app/Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::write(contents.join("MacOS/fakecraft"), b"#!/bin/sh\n").unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>\n"),
        )
        .unwrap();
        std::os::unix::fs::symlink("/Applications", src.join("Applications")).ok();
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
    let l = layout(&root);
    let active = root.join("apps/FakeCraft.app");
    let v1 = make_dmg("v1.dmg", "ai.storyteller.fakecraft", "1.2.0");
    let v2 = make_dmg("v2.dmg", "ai.storyteller.fakecraft", "1.3.0");

    assert_eq!(install(&l, &at("1.2.0"), PackageKind::Dmg, &v1).unwrap(), active);
    assert_eq!(bundle_version(&active).as_deref(), Some("1.2.0"));
    // The images were detached and their mount points removed.
    assert_eq!(std::fs::read_dir(&l.downloads).unwrap().count(), 0);
    assert_eq!(verify_signature(PackageKind::Dmg, &active).unwrap(), Trust::Unsigned);
    assert!(matches!(install(&l, &at("1.2.0"), PackageKind::Dmg, &v1), Err(Error::Exists(_))));
    assert!(!root.join("kept/fakecraft/1.2.0").exists(), "a version that can't be activated isn't left behind");

    // Update: placed in the kept folder, then swapped in; the old one moves out to its own folder.
    let placed = place(&l, &at("1.3.0"), PackageKind::Dmg, &v2).unwrap();
    assert_eq!(placed, root.join("kept/fakecraft/1.3.0/FakeCraft.app"));
    let a = activate(&l, &at("1.3.0"), PackageKind::Dmg, &placed, Some(Current { path: &active, version: "1.2.0" })).unwrap();
    let old = root.join("kept/fakecraft/1.2.0/FakeCraft.app");
    assert_eq!((a.active.as_path(), a.previous.as_deref()), (active.as_path(), Some(old.as_path())));
    assert_eq!((bundle_version(&active).as_deref(), bundle_version(&old).as_deref()), (Some("1.3.0"), Some("1.2.0")));
    assert!(!root.join("kept/fakecraft/1.3.0").exists());
    assert_eq!(find_unmanaged(&l, &app(), PackageKind::Dmg).into_iter().map(|f| f.version).collect::<Vec<_>>(), ["1.3.0"]);

    // Roll back, and forward again.
    let back = activate(&l, &at("1.2.0"), PackageKind::Dmg, &old, Some(Current { path: &active, version: "1.3.0" })).unwrap();
    assert_eq!(bundle_version(&active).as_deref(), Some("1.2.0"));
    let newer = back.previous.unwrap();
    assert_eq!(bundle_version(&newer).as_deref(), Some("1.3.0"));
    activate(&l, &at("1.3.0"), PackageKind::Dmg, &newer, Some(Current { path: &active, version: "1.2.0" })).unwrap();
    assert_eq!(bundle_version(&active).as_deref(), Some("1.3.0"));

    // Pruning removes kept versions only; uninstall removes the active one.
    assert!(matches!(remove_version(&l, &at("1.3.0"), PackageKind::Dmg, &active), Err(Error::Refused(_))));
    remove_version(&l, &at("1.2.0"), PackageKind::Dmg, &old).unwrap();
    assert!(!root.join("kept/fakecraft").exists());
    uninstall(&l, &at("1.3.0"), PackageKind::Dmg, &active).unwrap();
    assert!(!active.exists());

    // An app installed by hand is never replaced: activation leaves it and the kept one alone.
    let mine = make_dmg("mine.dmg", "ai.storyteller.fakecraft", "1.0.0");
    install(&l, &at("1.0.0"), PackageKind::Dmg, &mine).unwrap();
    let placed = place(&l, &at("1.3.0"), PackageKind::Dmg, &v2).unwrap();
    assert!(matches!(activate(&l, &at("1.3.0"), PackageKind::Dmg, &placed, None), Err(Error::Exists(_))));
    assert_eq!(bundle_version(&active).as_deref(), Some("1.0.0"));
    assert!(placed.exists());

    // A disk image holding another app is refused.
    let wrong = make_dmg("wrong.dmg", "com.example.other", "1.3.0");
    let r = place(&l, &at("1.4.0"), PackageKind::Dmg, &wrong);
    assert!(matches!(r, Err(Error::BadPackage(ref m)) if m.contains("com.example.other")), "{r:?}");
    assert!(!root.join("kept/fakecraft/1.4.0/FakeCraft.app").exists());
    let _ = std::fs::remove_dir_all(&root);
}
