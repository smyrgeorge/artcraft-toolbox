//! Who made an installed version, as the platform sees it: `codesign` and Gatekeeper (`spctl`) on
//! macOS, Authenticode on Windows. AppImages carry no platform signature.
//!
//! A signature that is present but broken means the files were changed after signing: that is an
//! error ([`Error::BadPackage`]), and the version is not activated. A missing signature is
//! reported, not refused (the toolbox shows it); a check that can't run says why.
//!
//! macOS uses `codesign --verify --deep`, not `--strict`: the crafts' disk images leave Finder
//! information on some files, which `--strict` rejects although the signature is intact and
//! Gatekeeper accepts the app (measured on PhotoCraft 0.3.0 and 0.5.0). Without `--strict` a
//! changed resource or executable is still caught.

use std::path::Path;
use std::process::{Command, Stdio};

use artcraft_toolbox_model::Trust;
use artcraft_toolbox_release::PackageKind;

use crate::{Error, Result};

/// Longest signer name kept (certificate names are short; the rest is noise).
const MAX_SIGNER: usize = 200;

/// Check the platform signature of the version at `path` (the `.app`, the `.exe`, the AppImage).
pub fn verify(kind: PackageKind, path: &Path) -> Result<Trust> {
    match kind {
        PackageKind::Dmg if cfg!(target_os = "macos") => macos(path),
        PackageKind::PortableZip if cfg!(windows) => windows(path),
        PackageKind::AppImage => Ok(Trust::Unsigned),
        PackageKind::Dmg => Ok(Trust::Unchecked { reason: "code signatures can only be checked on macOS".into() }),
        PackageKind::PortableZip => Ok(Trust::Unchecked { reason: "Authenticode can only be checked on Windows".into() }),
        other => Err(Error::Unsupported(other)),
    }
}

fn tool(cmd: &mut Command) -> std::io::Result<(bool, String)> {
    let out = cmd.stdin(Stdio::null()).output()?;
    // codesign and spctl report on stderr.
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    Ok((out.status.success(), text))
}

fn macos(app: &Path) -> Result<Trust> {
    let unchecked = |what: &str, e: std::io::Error| Ok(Trust::Unchecked { reason: format!("{what}: {e}") });
    let (valid, out) = match tool(Command::new("codesign").args(["--verify", "--deep"]).arg(app)) {
        Ok(r) => r,
        Err(e) => return unchecked("codesign", e),
    };
    if !valid {
        return verify_failure(&out);
    }
    let (_, details) = match tool(Command::new("codesign").args(["-dv", "--verbose=2"]).arg(app)) {
        Ok(r) => r,
        Err(e) => return unchecked("codesign", e),
    };
    let Some((signer, team)) = signer(&details) else { return Ok(Trust::Unsigned) };
    let notarized = match tool(Command::new("spctl").args(["--assess", "--type", "execute", "-vv"]).arg(app)) {
        Ok((accepted, out)) => gatekeeper_accepts(accepted, &out),
        Err(_) => false,
    };
    Ok(Trust::Signed { signer, team, notarized })
}

/// `codesign --verify` failed: unsigned, or a signature that no longer matches the files.
pub fn verify_failure(output: &str) -> Result<Trust> {
    if output.contains("is not signed at all") {
        return Ok(Trust::Unsigned);
    }
    let first = output.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no details");
    // `/long/path/PhotoCraft.app: a sealed resource is missing or invalid`: the reason is the point.
    let reason = first.split_once(".app: ").map_or(first, |(_, r)| r);
    Err(Error::BadPackage(format!("its code signature is broken: {}", reason.chars().take(MAX_SIGNER).collect::<String>())))
}

/// The signing certificate (first `Authority=`) and team from `codesign -dv` output; `None` for
/// ad-hoc signatures, which name nobody.
pub fn signer(details: &str) -> Option<(String, Option<String>)> {
    let value = |key: &str| details.lines().find_map(|l| l.trim().strip_prefix(key)).map(|v| v.trim().chars().take(MAX_SIGNER).collect::<String>());
    if value("Signature=").is_some_and(|s| s == "adhoc") {
        return None;
    }
    let authority = value("Authority=")?;
    let team = value("TeamIdentifier=").filter(|t| !t.is_empty() && t != "not set");
    Some((authority, team))
}

/// Gatekeeper accepted the app (`spctl --assess` exits 0 and says "accepted").
pub fn gatekeeper_accepts(exit_ok: bool, output: &str) -> bool {
    exit_ok && output.contains("accepted")
}

fn windows(exe: &Path) -> Result<Trust> {
    // The path travels in an environment variable, so no quoting can break the command. Only
    // stdout is read, its lines marked: Windows PowerShell may write progress records ("#< CLIXML")
    // to stderr when it loads a module for the first time. The cmdlet's module is imported from
    // Windows PowerShell's own folder: a `PSModulePath` inherited from PowerShell 7 (its terminal,
    // GitHub's runners) would load PowerShell 7's copy, which Windows PowerShell can't. A failure
    // comes back as an `ERROR=` line.
    let script = "$ProgressPreference = 'SilentlyContinue'; try { Import-Module (Join-Path $PSHOME 'Modules\\Microsoft.PowerShell.Security\\Microsoft.PowerShell.Security.psd1') -ErrorAction Stop; $s = Get-AuthenticodeSignature -LiteralPath $env:TOOLBOX_PATH -ErrorAction Stop; 'STATUS=' + $s.Status; if ($s.SignerCertificate) { 'SIGNER=' + $s.SignerCertificate.Subject } } catch { 'ERROR=' + $_.Exception.Message }";
    let out = crate::powershell(script).env("TOOLBOX_PATH", exe).stdin(Stdio::null()).output();
    match out {
        Ok(out) => authenticode(&String::from_utf8_lossy(&out.stdout)),
        Err(e) => Ok(Trust::Unchecked { reason: format!("powershell: {e}") }),
    }
}

/// The `STATUS=` and `SIGNER=` lines of `Get-AuthenticodeSignature`'s result, or the `ERROR=`
/// line of a check that couldn't run; other lines are ignored.
pub fn authenticode(output: &str) -> Result<Trust> {
    let value = |key: &str| output.lines().find_map(|l| l.trim().strip_prefix(key)).map(str::trim);
    if let Some(e) = value("ERROR=") {
        return Ok(Trust::Unchecked { reason: format!("Authenticode: {}", e.chars().take(MAX_SIGNER).collect::<String>()) });
    }
    let status = value("STATUS=").unwrap_or("");
    let subject = value("SIGNER=").filter(|s| !s.is_empty()).map(|s| s.chars().take(MAX_SIGNER).collect::<String>());
    match (status, subject) {
        ("Valid", Some(signer)) => Ok(Trust::Signed { signer, team: None, notarized: false }),
        ("NotSigned", _) => Ok(Trust::Unsigned),
        ("HashMismatch", _) => Err(Error::BadPackage("its Authenticode signature doesn't match its contents".into())),
        (other, _) => Ok(Trust::Unchecked { reason: format!("Authenticode status {}", if other.is_empty() { "unknown" } else { other }) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codesign_details() {
        let photocraft = "Executable=/Applications/PhotoCraft.app/Contents/MacOS/photocraft\nIdentifier=ai.storyteller.photocraft\nFormat=app bundle with Mach-O universal (x86_64 arm64)\nAuthority=Developer ID Application: Learning Machines LLC (DJ6XS33FX8)\nAuthority=Developer ID Certification Authority\nAuthority=Apple Root CA\nTimestamp=26 Sep 2026\nTeamIdentifier=DJ6XS33FX8\n";
        assert_eq!(signer(photocraft), Some(("Developer ID Application: Learning Machines LLC (DJ6XS33FX8)".into(), Some("DJ6XS33FX8".into()))));
        let adhoc = "Executable=/x\nIdentifier=x\nSignature=adhoc\nTeamIdentifier=not set\n";
        assert_eq!(signer(adhoc), None);
        let apple = "Authority=Software Signing\nAuthority=Apple Code Signing Certification Authority\nTeamIdentifier=not set\n";
        assert_eq!(signer(apple), Some(("Software Signing".into(), None)));
        assert_eq!(signer(""), None);
    }

    #[test]
    fn broken_and_missing_signatures() {
        assert_eq!(verify_failure("/x/Fake.app: code object is not signed at all\n").unwrap(), Trust::Unsigned);
        let broken = verify_failure("/a/very/long/path/PhotoCraft.app: a sealed resource is missing or invalid\nfile modified: /x/a\n");
        assert!(matches!(&broken, Err(Error::BadPackage(m)) if m == "its code signature is broken: a sealed resource is missing or invalid"), "{broken:?}");
        let patched = verify_failure("/x/PhotoCraft.app: invalid signature (code or signature have been modified)\nIn architecture: x86_64\n");
        assert!(
            matches!(&patched, Err(Error::BadPackage(m)) if m.ends_with("broken: invalid signature (code or signature have been modified)")),
            "{patched:?}"
        );
        assert!(gatekeeper_accepts(true, "/x/PhotoCraft.app: accepted\nsource=Notarized Developer ID\n"));
        assert!(!gatekeeper_accepts(false, "/x/PhotoCraft.app: rejected\nsource=Unnotarized Developer ID\n"));
    }

    #[test]
    fn authenticode_statuses() {
        assert_eq!(
            authenticode("STATUS=Valid\r\nSIGNER=CN=Learning Machines LLC, O=Learning Machines LLC\r\n").unwrap(),
            Trust::Signed { signer: "CN=Learning Machines LLC, O=Learning Machines LLC".into(), team: None, notarized: false }
        );
        // Anything else PowerShell prints around the marked lines is ignored.
        assert_eq!(
            authenticode("#< CLIXML\r\n<Objs Version=\"1.1.0.1\"></Objs>\r\nSTATUS=Valid\r\nSIGNER=CN=Microsoft Windows\r\n").unwrap(),
            Trust::Signed { signer: "CN=Microsoft Windows".into(), team: None, notarized: false }
        );
        assert_eq!(authenticode("STATUS=NotSigned\r\n").unwrap(), Trust::Unsigned);
        assert!(matches!(authenticode("STATUS=HashMismatch\nSIGNER=CN=x\n"), Err(Error::BadPackage(_))));
        assert_eq!(authenticode("STATUS=NotTrusted\nSIGNER=CN=x\n").unwrap(), Trust::Unchecked { reason: "Authenticode status NotTrusted".into() });
        assert_eq!(
            authenticode("STATUS=Valid\n").unwrap(),
            Trust::Unchecked { reason: "Authenticode status Valid".into() },
            "valid but no signer: not trusted blindly"
        );
        assert_eq!(authenticode("").unwrap(), Trust::Unchecked { reason: "Authenticode status unknown".into() });
        assert_eq!(
            authenticode("ERROR=The module could not be loaded.\r\n").unwrap(),
            Trust::Unchecked { reason: "Authenticode: The module could not be loaded.".into() }
        );
    }

    #[test]
    fn appimages_have_no_platform_signature() {
        assert_eq!(verify(PackageKind::AppImage, Path::new("/x")).unwrap(), Trust::Unsigned);
        assert!(matches!(verify(PackageKind::Msi, Path::new("/x")), Err(Error::Unsupported(_))));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_system_app_is_signed_and_accepted() {
        let calc = Path::new("/System/Applications/Calculator.app");
        if !calc.exists() {
            return;
        }
        match verify(PackageKind::Dmg, calc).unwrap() {
            Trust::Signed { signer, .. } => assert!(!signer.is_empty()),
            other => panic!("Calculator: {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_system_files_are_signed() {
        let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
        let trust = verify(PackageKind::PortableZip, cmd).unwrap();
        assert!(matches!(trust, Trust::Signed { .. }), "cmd.exe: {trust:?}");
        let plain = std::env::temp_dir().join(format!("artcraft-toolbox-trust-{}.exe", std::process::id()));
        std::fs::write(&plain, b"not a program").unwrap();
        let trust = verify(PackageKind::PortableZip, &plain).unwrap();
        assert!(!matches!(trust, Trust::Signed { .. }), "a file that isn't a program: {trust:?}");
        let _ = std::fs::remove_file(&plain);
    }
}
