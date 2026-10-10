//! Release asset file names: `<slug>-<version>-<os>-<arch>.<ext>` and friends.
//!
//! Every Crafting App publishes the same set (`docs/release-contract.md`):
//!
//! ```text
//! photocraft-0.5.0-macos-universal.dmg          photocraft-0.5.0-linux-x86_64.AppImage(.zsync)
//! photocraft-0.5.0-windows-x64.msi              photocraft-0.5.0-linux-x86_64.{deb,rpm,tar.gz,flatpak}
//! photocraft-0.5.0-windows-x64-portable.zip     photocraft-0.5.0-freebsd-x86_64.tar.gz
//! photocraft-cli-0.5.0-macos-universal.zip      photocraft-web-0.5.0.zip
//! ```
//!
//! Names are parsed from the right (extension, arch, os), so a pre-release version with its own
//! hyphens (`1.0.0-rc.1`) is never mistaken for the platform part.

use serde::{Deserialize, Serialize};

use crate::{Arch, Os, Target, Version};

/// Longest asset name considered; anything longer is not one of ours.
pub const MAX_NAME_LEN: usize = 256;

/// Which program an asset holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Component {
    /// The desktop app.
    App,
    /// The headless CLI (`<slug>-cli-…`).
    Cli,
    /// The static web build (`<slug>-web-<version>.zip`).
    Web,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    Dmg,
    Msi,
    /// Windows portable zip (`…-portable.zip`).
    PortableZip,
    Zip,
    AppImage,
    /// AppImage delta-update metadata; never installed.
    AppImageZsync,
    Deb,
    Rpm,
    TarGz,
    Flatpak,
}

/// Extensions, longest first so `.AppImage.zsync` wins over `.AppImage`.
const EXTENSIONS: &[(&str, PackageKind)] = &[
    (".AppImage.zsync", PackageKind::AppImageZsync),
    (".AppImage", PackageKind::AppImage),
    (".flatpak", PackageKind::Flatpak),
    (".tar.gz", PackageKind::TarGz),
    (".dmg", PackageKind::Dmg),
    (".msi", PackageKind::Msi),
    (".deb", PackageKind::Deb),
    (".rpm", PackageKind::Rpm),
    (".zip", PackageKind::Zip),
];

impl PackageKind {
    /// What the toolbox installs on `os`, best first: per-user packages that need no admin
    /// rights and can sit side by side (one directory per version, so rollback is a switch).
    pub fn preferred(os: Os) -> &'static [PackageKind] {
        match os {
            Os::Macos => &[PackageKind::Dmg],
            Os::Windows => &[PackageKind::PortableZip, PackageKind::Msi],
            Os::Linux => &[PackageKind::AppImage, PackageKind::TarGz],
            Os::Freebsd => &[PackageKind::TarGz],
        }
    }
}

/// A parsed release asset name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetName {
    /// The slug the name started with: the app id, or one of its former names (`printcraft`).
    pub slug: String,
    pub component: Component,
    pub version: Version,
    /// `None` for the platform-independent web build.
    pub target: Option<Target>,
    pub kind: PackageKind,
}

/// Longest file-name pattern of an app outside the contract (`catalog.toml` › `[app.assets]`).
pub const MAX_PATTERN_LEN: usize = 128;
/// What a pattern writes where the version goes: `ArtCraft_{version}_universal.dmg`.
pub const VERSION_PLACEHOLDER: &str = "{version}";

/// Match `file` against a catalog pattern of an app outside the contract, for the build
/// `target`: the pattern with `{version}` written out must be the whole file name. The package
/// kind comes from the extension, as in the contract.
pub fn from_pattern(file: &str, pattern: &str, version: &Version, target: Target, slug: &str) -> Option<AssetName> {
    if file.len() > MAX_NAME_LEN || pattern.len() > MAX_PATTERN_LEN || !pattern.contains(VERSION_PLACEHOLDER) {
        return None;
    }
    if file != pattern.replace(VERSION_PLACEHOLDER, &version.to_string()) {
        return None;
    }
    let (_, kind) = EXTENSIONS.iter().find(|(ext, _)| file.ends_with(ext))?;
    let kind = if *kind == PackageKind::Zip && file.ends_with("-portable.zip") { PackageKind::PortableZip } else { *kind };
    Some(AssetName { slug: slug.to_string(), component: Component::App, version: version.clone(), target: Some(target), kind })
}

/// Parse `file` as an asset of an app published under any of `slugs`. `None` for names that
/// don't follow the contract (including `SHA256SUMS.txt`).
pub fn parse(file: &str, slugs: &[&str]) -> Option<AssetName> {
    if file.len() > MAX_NAME_LEN {
        return None;
    }
    let (slug, rest) = slugs.iter().filter(|s| !s.is_empty()).find_map(|s| Some((*s, file.strip_prefix(s)?.strip_prefix('-')?)))?;
    let (component, rest) = if let Some(r) = rest.strip_prefix("cli-") {
        (Component::Cli, r)
    } else if let Some(r) = rest.strip_prefix("web-") {
        (Component::Web, r)
    } else {
        (Component::App, rest)
    };
    let (stem, mut kind) = EXTENSIONS.iter().find_map(|(ext, k)| Some((rest.strip_suffix(ext)?, *k)))?;
    let stem = match (kind, stem.strip_suffix("-portable")) {
        (PackageKind::Zip, Some(s)) => {
            kind = PackageKind::PortableZip;
            s
        }
        _ => stem,
    };
    if component == Component::Web {
        let version = Version::parse(stem).ok()?;
        return Some(AssetName { slug: slug.to_string(), component, version, target: None, kind });
    }
    let (stem, arch) = stem.rsplit_once('-')?;
    let (version, os) = stem.rsplit_once('-')?;
    let target = Target::new(Os::from_token(os)?, Arch::from_token(arch)?);
    let version = Version::parse(version).ok()?;
    Some(AssetName { slug: slug.to_string(), component, version, target: Some(target), kind })
}

/// The desktop-app asset to install on `host` among `items`: the most native architecture first,
/// then the preferred package kind ([`PackageKind::preferred`]).
pub fn select<T>(items: &[T], host: Target, name: impl Fn(&T) -> &AssetName) -> Option<&T> {
    host.runnable_archs().iter().find_map(|arch| {
        PackageKind::preferred(host.os).iter().find_map(|kind| {
            items.iter().find(|item| {
                let n = name(item);
                n.component == Component::App && n.kind == *kind && n.target == Some(Target::new(host.os, *arch))
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLUGS: &[&str] = &["photocraft"];

    fn p(name: &str) -> AssetName {
        parse(name, SLUGS).unwrap_or_else(|| panic!("{name} should parse"))
    }

    /// The asset list of PhotoCraft v0.5.0 as published on GitHub (2026-10-08).
    const PHOTOCRAFT_050: &[&str] = &[
        "photocraft-0.5.0-freebsd-x86_64.tar.gz",
        "photocraft-0.5.0-linux-aarch64.AppImage",
        "photocraft-0.5.0-linux-aarch64.AppImage.zsync",
        "photocraft-0.5.0-linux-aarch64.deb",
        "photocraft-0.5.0-linux-aarch64.flatpak",
        "photocraft-0.5.0-linux-aarch64.rpm",
        "photocraft-0.5.0-linux-aarch64.tar.gz",
        "photocraft-0.5.0-linux-x86_64.AppImage",
        "photocraft-0.5.0-linux-x86_64.AppImage.zsync",
        "photocraft-0.5.0-linux-x86_64.deb",
        "photocraft-0.5.0-linux-x86_64.flatpak",
        "photocraft-0.5.0-linux-x86_64.rpm",
        "photocraft-0.5.0-linux-x86_64.tar.gz",
        "photocraft-0.5.0-macos-universal.dmg",
        "photocraft-0.5.0-windows-arm64-portable.zip",
        "photocraft-0.5.0-windows-arm64.msi",
        "photocraft-0.5.0-windows-x64-portable.zip",
        "photocraft-0.5.0-windows-x64.msi",
        "photocraft-0.5.0-windows-x86-portable.zip",
        "photocraft-0.5.0-windows-x86.msi",
        "photocraft-cli-0.5.0-macos-universal.zip",
        "photocraft-web-0.5.0.zip",
    ];

    #[test]
    fn every_published_asset_parses() {
        for name in PHOTOCRAFT_050 {
            let a = p(name);
            assert_eq!(a.version, Version::new(0, 5, 0), "{name}");
            assert_eq!(a.slug, "photocraft");
        }
        assert_eq!(parse("SHA256SUMS.txt", SLUGS), None);
    }

    #[test]
    fn fields() {
        let a = p("photocraft-0.5.0-windows-x64-portable.zip");
        assert_eq!((a.component, a.kind, a.target), (Component::App, PackageKind::PortableZip, Some(Target::new(Os::Windows, Arch::X86_64))));
        let a = p("photocraft-cli-0.5.0-macos-universal.zip");
        assert_eq!((a.component, a.kind, a.target), (Component::Cli, PackageKind::Zip, Some(Target::new(Os::Macos, Arch::Universal))));
        let a = p("photocraft-web-0.5.0.zip");
        assert_eq!((a.component, a.kind, a.target), (Component::Web, PackageKind::Zip, None));
        assert_eq!(p("photocraft-0.5.0-linux-x86_64.AppImage.zsync").kind, PackageKind::AppImageZsync);
    }

    #[test]
    fn prerelease_versions_keep_their_hyphens() {
        let a = p("photocraft-1.0.0-rc.1-linux-x86_64.AppImage");
        assert_eq!(a.version.to_string(), "1.0.0-rc.1");
        let a = p("photocraft-1.0.0-x-y.2-windows-arm64-portable.zip");
        assert_eq!(a.version.to_string(), "1.0.0-x-y.2");
        assert_eq!(a.target, Some(Target::new(Os::Windows, Arch::Aarch64)));
    }

    #[test]
    fn former_slugs() {
        // PdfCraft published 0.2.x as PrintCraft.
        let a = parse("printcraft-0.2.1-linux-x86_64.AppImage", &["pdfcraft", "printcraft"]).unwrap();
        assert_eq!(a.slug, "printcraft");
        assert_eq!(parse("printcraft-0.2.1-linux-x86_64.AppImage", &["pdfcraft"]), None);
    }

    #[test]
    fn rejects_foreign_and_hostile_names() {
        for bad in [
            "",
            "photocraft",
            "photocraft-",
            "photocraft-0.5.0.dmg",
            "photocraft-0.5.0-macos.dmg",
            "photocraft-0.5.0-plan9-x86_64.tar.gz",
            "photocraft-0.5.0-linux-sparc.tar.gz",
            "photocraft-latest-linux-x86_64.AppImage",
            "photocraft-0.5.0-linux-x86_64.exe",
            "photocraftx-0.5.0-linux-x86_64.AppImage",
            "vectorcraft-0.5.0-linux-x86_64.AppImage",
            "ArtCraft_0.41.0_universal.dmg",
            "photocraft-web-0.5.0-linux-x86_64.zip",
        ] {
            assert_eq!(parse(bad, SLUGS), None, "{bad}");
        }
        let long = format!("photocraft-0.5.0-linux-x86_64{}.AppImage", "-".repeat(MAX_NAME_LEN));
        assert_eq!(parse(&long, SLUGS), None);
        assert_eq!(parse("photocraft-0.5.0-linux-x86_64.AppImage", &[""]), None);
    }

    fn all() -> Vec<AssetName> {
        PHOTOCRAFT_050.iter().map(|n| p(n)).collect()
    }

    fn pick(os: Os, arch: Arch) -> Option<String> {
        let all = all();
        select(&all, Target::new(os, arch), |a| a).map(|a| format!("{}-{:?}", a.target.map(|t| t.to_string()).unwrap_or_default(), a.kind))
    }

    #[test]
    fn selects_the_per_user_package_for_each_host() {
        assert_eq!(pick(Os::Macos, Arch::Aarch64).as_deref(), Some("macos-universal-Dmg"));
        assert_eq!(pick(Os::Macos, Arch::X86_64).as_deref(), Some("macos-universal-Dmg"));
        assert_eq!(pick(Os::Windows, Arch::X86_64).as_deref(), Some("windows-x86_64-PortableZip"));
        assert_eq!(pick(Os::Windows, Arch::Aarch64).as_deref(), Some("windows-aarch64-PortableZip"));
        assert_eq!(pick(Os::Linux, Arch::X86_64).as_deref(), Some("linux-x86_64-AppImage"));
        assert_eq!(pick(Os::Linux, Arch::Aarch64).as_deref(), Some("linux-aarch64-AppImage"));
        assert_eq!(pick(Os::Freebsd, Arch::X86_64).as_deref(), Some("freebsd-x86_64-TarGz"));
        assert_eq!(pick(Os::Freebsd, Arch::Aarch64), None);
    }

    #[test]
    fn falls_back_to_emulated_architectures() {
        // Windows on ARM before the craft shipped arm64 builds (0.2.0): run the x64 build.
        let old: Vec<AssetName> = ["photocraft-0.2.0-windows-x64-portable.zip", "photocraft-0.2.0-windows-x86.msi"].iter().map(|n| p(n)).collect();
        let a = select(&old, Target::new(Os::Windows, Arch::Aarch64), |a| a).unwrap();
        assert_eq!(a.target, Some(Target::new(Os::Windows, Arch::X86_64)));
        // Only the MSI for x86: a 64-bit Windows still takes it rather than nothing.
        let msi_only: Vec<AssetName> = ["photocraft-0.2.0-windows-x86.msi"].iter().map(|n| p(n)).collect();
        assert_eq!(select(&msi_only, Target::new(Os::Windows, Arch::X86_64), |a| a).map(|a| a.kind), Some(PackageKind::Msi));
    }

    #[test]
    fn never_selects_cli_web_or_zsync() {
        let only: Vec<AssetName> = ["photocraft-cli-0.5.0-macos-universal.zip", "photocraft-web-0.5.0.zip", "photocraft-0.5.0-linux-x86_64.AppImage.zsync"]
            .iter()
            .map(|n| p(n))
            .collect();
        for (os, arch) in [(Os::Macos, Arch::Aarch64), (Os::Linux, Arch::X86_64)] {
            assert!(select(&only, Target::new(os, arch), |a| a).is_none());
        }
    }
}

#[cfg(test)]
mod pattern_tests {
    use super::*;

    #[test]
    fn patterns_match_whole_names_and_take_the_kind_from_the_extension() {
        let v = Version::new(0, 41, 0);
        let mac = Target::from_tokens("macos-universal").unwrap();
        let a = from_pattern("ArtCraft_0.41.0_universal.dmg", "ArtCraft_{version}_universal.dmg", &v, mac, "artcraft").unwrap();
        assert_eq!((a.slug.as_str(), a.component, &a.version, a.target, a.kind), ("artcraft", Component::App, &v, Some(mac), PackageKind::Dmg));
        let win = Target::from_tokens("windows-x64").unwrap();
        assert_eq!(from_pattern("App_0.41.0_x64-portable.zip", "App_{version}_x64-portable.zip", &v, win, "app").unwrap().kind, PackageKind::PortableZip);
        assert_eq!(from_pattern("App_0.41.0_x64.msi", "App_{version}_x64.msi", &v, win, "app").unwrap().kind, PackageKind::Msi);
        // Another version, a prefix, a suffix, an unknown extension, a pattern without the placeholder.
        assert!(from_pattern("ArtCraft_0.40.0_universal.dmg", "ArtCraft_{version}_universal.dmg", &v, mac, "artcraft").is_none());
        assert!(from_pattern("xArtCraft_0.41.0_universal.dmg", "ArtCraft_{version}_universal.dmg", &v, mac, "artcraft").is_none());
        assert!(from_pattern("ArtCraft_0.41.0_universal.dmg.sig", "ArtCraft_{version}_universal.dmg", &v, mac, "artcraft").is_none());
        assert!(from_pattern("ArtCraft_0.41.0_x64-setup.exe", "ArtCraft_{version}_x64-setup.exe", &v, win, "artcraft").is_none());
        assert!(from_pattern("ArtCraft.dmg", "ArtCraft.dmg", &v, mac, "artcraft").is_none());
        assert!(from_pattern(&"a".repeat(MAX_NAME_LEN + 1), "a{version}", &v, mac, "a").is_none());
        assert!(Target::from_tokens("macos").is_none() && Target::from_tokens("amiga-m68k").is_none());
    }
}
