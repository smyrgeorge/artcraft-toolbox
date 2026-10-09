//! Operating systems and CPU architectures, as release asset names spell them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Windows,
    Linux,
    Freebsd,
}

impl Os {
    /// The token in asset names (`photocraft-0.5.0-macos-universal.dmg`).
    pub fn token(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Windows => "windows",
            Os::Linux => "linux",
            Os::Freebsd => "freebsd",
        }
    }

    pub fn from_token(s: &str) -> Option<Os> {
        Some(match s {
            "macos" => Os::Macos,
            "windows" => Os::Windows,
            "linux" => Os::Linux,
            "freebsd" => Os::Freebsd,
            _ => return None,
        })
    }

    /// User-facing name.
    pub fn label(self) -> &'static str {
        match self {
            Os::Macos => "macOS",
            Os::Windows => "Windows",
            Os::Linux => "Linux",
            Os::Freebsd => "FreeBSD",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    X86_64,
    Aarch64,
    X86,
    /// A macOS universal binary (Apple silicon + Intel in one file).
    Universal,
}

impl Arch {
    /// The canonical token. Linux and FreeBSD assets use it; Windows assets use [`Arch::from_token`]'s
    /// aliases (`x64`, `arm64`).
    pub fn token(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
            Arch::X86 => "x86",
            Arch::Universal => "universal",
        }
    }

    pub fn from_token(s: &str) -> Option<Arch> {
        Some(match s {
            "x86_64" | "x64" | "amd64" => Arch::X86_64,
            "aarch64" | "arm64" => Arch::Aarch64,
            "x86" | "i686" => Arch::X86,
            "universal" => Arch::Universal,
            _ => return None,
        })
    }
}

/// An OS and architecture pair: where a build runs, or what an asset was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Target {
    pub os: Os,
    pub arch: Arch,
}

impl Target {
    pub const fn new(os: Os, arch: Arch) -> Target {
        Target { os, arch }
    }

    /// The machine this process runs on; `None` on a platform no craft ships for.
    pub fn host() -> Option<Target> {
        Target::from_consts(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// From `std::env::consts::{OS, ARCH}` values.
    pub fn from_consts(os: &str, arch: &str) -> Option<Target> {
        let os = Os::from_token(os)?;
        let arch = match arch {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            "x86" => Arch::X86,
            _ => return None,
        };
        Some(Target { os, arch })
    }

    /// Asset architectures that run here, best first: native, then what the OS emulates
    /// (Rosetta 2 on Apple silicon, x64 emulation on Windows on ARM, WoW64 for 32-bit).
    pub fn runnable_archs(self) -> &'static [Arch] {
        match (self.os, self.arch) {
            (Os::Macos, Arch::Aarch64) => &[Arch::Universal, Arch::Aarch64, Arch::X86_64],
            (Os::Macos, _) => &[Arch::Universal, Arch::X86_64],
            (Os::Windows, Arch::Aarch64) => &[Arch::Aarch64, Arch::X86_64, Arch::X86],
            (Os::Windows, Arch::X86_64) => &[Arch::X86_64, Arch::X86],
            (Os::Windows, _) => &[Arch::X86],
            (_, Arch::Aarch64) => &[Arch::Aarch64],
            (_, Arch::X86) => &[Arch::X86],
            (_, _) => &[Arch::X86_64],
        }
    }

    /// Can an asset built for `asset` run on this machine?
    pub fn runs(self, asset: Target) -> bool {
        asset.os == self.os && self.runnable_archs().contains(&asset.arch)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.os.token(), self.arch.token())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        for os in [Os::Macos, Os::Windows, Os::Linux, Os::Freebsd] {
            assert_eq!(Os::from_token(os.token()), Some(os));
        }
        for a in [Arch::X86_64, Arch::Aarch64, Arch::X86, Arch::Universal] {
            assert_eq!(Arch::from_token(a.token()), Some(a));
        }
        assert_eq!(Arch::from_token("x64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_token("arm64"), Some(Arch::Aarch64));
        assert_eq!(Os::from_token("MacOS"), None);
    }

    #[test]
    fn host_is_known_on_ci_platforms() {
        // macOS, Windows and Linux runners are all x86_64 or aarch64.
        assert!(Target::host().is_some());
        assert_eq!(Target::from_consts("linux", "riscv64"), None);
        assert_eq!(Target::from_consts("haiku", "x86_64"), None);
    }

    #[test]
    fn universal_runs_on_every_mac_and_only_on_macs() {
        let uni = Target::new(Os::Macos, Arch::Universal);
        assert!(Target::new(Os::Macos, Arch::Aarch64).runs(uni));
        assert!(Target::new(Os::Macos, Arch::X86_64).runs(uni));
        assert!(!Target::new(Os::Linux, Arch::X86_64).runs(uni));
        // Intel Macs can't run arm64-only builds.
        assert!(!Target::new(Os::Macos, Arch::X86_64).runs(Target::new(Os::Macos, Arch::Aarch64)));
    }

    #[test]
    fn windows_emulation_order() {
        let arm = Target::new(Os::Windows, Arch::Aarch64);
        assert_eq!(arm.runnable_archs().first(), Some(&Arch::Aarch64));
        assert!(arm.runs(Target::new(Os::Windows, Arch::X86_64)));
        assert!(!Target::new(Os::Windows, Arch::X86).runs(Target::new(Os::Windows, Arch::X86_64)));
        assert!(!Target::new(Os::Linux, Arch::Aarch64).runs(Target::new(Os::Linux, Arch::X86_64)));
    }
}
