//! Real GitHub "list releases" responses (trimmed: notes replaced, four releases each), captured
//! 2026-10-09. They pin the release contract as the crafts actually publish it.

use artcraft_toolbox_feed::{Status, parse_releases, status};
use artcraft_toolbox_model::Channel;
use artcraft_toolbox_release::{Arch, Os, PackageKind, Target, Version};

const PHOTOCRAFT: &str = include_str!("fixtures/photocraft-releases.json");
const PDFCRAFT: &str = include_str!("fixtures/pdfcraft-releases.json");

const HOSTS: &[(Os, Arch, PackageKind)] = &[
    (Os::Macos, Arch::Aarch64, PackageKind::Dmg),
    (Os::Macos, Arch::X86_64, PackageKind::Dmg),
    (Os::Windows, Arch::X86_64, PackageKind::PortableZip),
    (Os::Windows, Arch::Aarch64, PackageKind::PortableZip),
    (Os::Linux, Arch::X86_64, PackageKind::AppImage),
    (Os::Linux, Arch::Aarch64, PackageKind::AppImage),
    (Os::Freebsd, Arch::X86_64, PackageKind::TarGz),
];

#[test]
fn photocraft_feed_follows_the_contract() {
    let releases = parse_releases(PHOTOCRAFT, &["photocraft"]).unwrap();
    assert_eq!(releases.len(), 4);
    let newest = &releases[0];
    assert_eq!(newest.version, Version::new(0, 5, 0));
    assert!(newest.unrecognized.is_empty(), "{:?}", newest.unrecognized);
    assert!(newest.checksums_url.is_some());
    for (os, arch, kind) in HOSTS {
        let host = Target::new(*os, *arch);
        let (r, a) = artcraft_toolbox_feed::latest(&releases, Channel::Stable, host, None).unwrap_or_else(|| panic!("no build for {host}"));
        assert_eq!(r.version, Version::new(0, 5, 0), "{host}");
        assert_eq!(a.name.kind, *kind, "{host}");
        assert!(a.url.starts_with("https://github.com/storytold/photocraft/releases/download/v0.5.0/"), "{}", a.url);
    }
}

#[test]
fn pdfcraft_keeps_its_printcraft_releases() {
    let releases = parse_releases(PDFCRAFT, &["pdfcraft", "printcraft"]).unwrap();
    let old = releases.iter().find(|r| r.version == Version::new(0, 2, 1)).unwrap();
    assert!(!old.assets.is_empty());
    assert!(old.assets.iter().all(|a| a.name.slug == "printcraft"));
    // Without the former slug, those assets are not recognized.
    let strict = parse_releases(PDFCRAFT, &["pdfcraft"]).unwrap();
    let old = strict.iter().find(|r| r.version == Version::new(0, 2, 1)).unwrap();
    assert!(old.assets.is_empty() && !old.unrecognized.is_empty());
}

#[test]
fn update_from_an_old_install() {
    let releases = parse_releases(PHOTOCRAFT, &["photocraft"]).unwrap();
    let installed = Version::new(0, 2, 0);
    let s = status(Some(&installed), Some(&releases), Channel::Stable, Some(Target::new(Os::Linux, Arch::X86_64)), None);
    assert_eq!(s, Status::UpdateAvailable { installed, latest: Version::new(0, 5, 0) });
}
