//! Windows only: embed the app icon and version info (VERSIONINFO) into `artcraft-toolbox-cli.exe`
//! (PhotoCraft's `build.rs`, ported).
//!
//! On every other target this does nothing, even when `ARTCRAFT_TOOLBOX_REQUIRE_WINRES` is set. A
//! missing resource compiler is a warning, so a cross-compile from macOS or Linux still links,
//! unless `ARTCRAFT_TOOLBOX_REQUIRE_WINRES=1` (set by the release packaging script) turns it into
//! a build error.

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/artcraft-toolbox.ico");
    println!("cargo:rerun-if-env-changed=ARTCRAFT_TOOLBOX_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/artcraft-toolbox.ico")
        .set("ProductName", "ArtCraft Toolbox")
        .set("FileDescription", "ArtCraft Toolbox command-line interface")
        .set("CompanyName", "ArtCraft Toolbox contributors")
        .set("LegalCopyright", "Copyright (c) the ArtCraft Toolbox authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "artcraft-toolbox-cli.exe")
        .set("InternalName", "artcraft-toolbox-cli");
    if let Err(err) = res.compile() {
        if std::env::var_os("ARTCRAFT_TOOLBOX_REQUIRE_WINRES").is_some() {
            return Err(std::io::Error::other(format!(
                "embedding Windows resources into artcraft-toolbox-cli.exe failed: {err}. \
                 Install the Windows SDK resource compiler (rc.exe) or mingw-w64 windres and rebuild."
            )));
        }
        println!("cargo:warning=artcraft-toolbox-cli.exe built without icon/version resources: {err}");
    }
    Ok(())
}
