# Releasing ArtCraft Toolbox

Every push to the `release` branch runs `.github/workflows/release.yml`. The workflow builds
signed installers for macOS, Windows and Linux, then creates or updates a **draft** GitHub
Release named `ArtCraft Toolbox v<version>`. Nobody sees a draft until a maintainer publishes it.

This is PhotoCraft's release pipeline, ported (`../photocraft/.github/workflows/release.yml` and
`packaging/`; the canonical recipe is `../craftrules/release/playbook.md`), so the toolbox
**follows its own release contract** (`docs/release-contract.md`): the same parsers that read a
craft's releases read the toolbox's, which is how it updates itself (docs/architecture.md § 7).
User-facing names say **ArtCraft Toolbox**. Files, binaries and ids stay lowercase
(`artcraft-toolbox-<version>-<os>-<arch>.<ext>`, `ai.storyteller.toolbox`).

Dropped from PhotoCraft's pipeline, because the toolbox doesn't need them: the embedded fonts
(craft-fonts), HEIF, file associations, the Flatpak, FreeBSD and web jobs.

## Cutting a release

1. **Bump the version** on `main`. The only place it lives is `[workspace.package] version`
   in the root `Cargo.toml`:

   ```sh
   cargo xtask version                 # prints the current version, e.g. 0.1.0
   cargo xtask version set 0.2.0       # or 0.2.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   Commit the change (`Cargo.toml` + `Cargo.lock`) through the normal review flow.
2. **Merge `main` into `release`** (or fast-forward it) and push. The workflow starts by itself.
3. **Wait for the draft.** After about 20 to 40 minutes (notarization is the slow part), the
   Releases page has a draft `ArtCraft Toolbox v0.2.0`, tagged `v0.2.0` on the pushed commit,
   with every artifact and `SHA256SUMS.txt`. The notes are generated from the merged PRs.
4. **Check it.** Download an installer or two and look at the job summaries. Any
   `::warning::` there means a signing secret was missing and that artifact is unsigned. The
   `release` job has already run `cargo xtask contract --dir` over the artifacts: every name
   follows the contract, every host gets its package, every file is in `SHA256SUMS.txt`.
5. **Publish** the draft in the GitHub UI. Publishing creates the `v0.2.0` tag. Versions with a
   pre-release suffix (`-rc.1`) are marked as pre-releases, which the toolbox offers only on
   the Pre-release channel.

Pushing to `release` again before you publish rebuilds the same draft and replaces its assets.
After the draft is published, the workflow refuses to touch that version again, so bump it first.
Once published, every running toolbox finds the release at its next check and offers to update
itself (`toolbox.update`), so a published release must be right: the toolbox only accepts an
update **signed by the same developer** as the copy it replaces (`install::trust`); an unsigned
release can't replace a signed install.

**Test runs:** *Actions → Release → Run workflow* runs the whole pipeline by hand. The optional
`version` input (such as `0.2.0-rc.1`) overrides `Cargo.toml` for that run only. The jobs apply
it with `cargo xtask version set` before building, so the binaries report it too. The run still
needs the `release` environment, which only the `release` branch can use, so pick that branch
in the dialog.

## What gets built

| Platform | Artifacts | Built on |
|---|---|---|
| macOS 11+ (universal: Apple silicon + Intel) | `artcraft-toolbox-<v>-macos-universal.dmg`, `artcraft-toolbox-cli-<v>-macos-universal.zip` | `macos-15` |
| Windows 10+ x64 | `artcraft-toolbox-<v>-windows-x64.msi`, `artcraft-toolbox-<v>-windows-x64-portable.zip` | `windows-latest` |
| Windows 10+ x86 (32-bit) | `artcraft-toolbox-<v>-windows-x86.msi`, `artcraft-toolbox-<v>-windows-x86-portable.zip` | `windows-latest` |
| Windows 11 on ARM | `artcraft-toolbox-<v>-windows-arm64.msi`, `artcraft-toolbox-<v>-windows-arm64-portable.zip` | `windows-latest` (cross-compiled) |
| Linux x86_64 | `artcraft-toolbox-<v>-linux-x86_64.{AppImage,AppImage.zsync,deb,rpm,tar.gz}` | `ubuntu-22.04` |
| Linux aarch64 | `artcraft-toolbox-<v>-linux-aarch64.{AppImage,AppImage.zsync,deb,rpm,tar.gz}` | `ubuntu-22.04-arm` |

Which of them the toolbox installs for itself: the DMG on macOS, the portable zip on Windows and
the AppImage on Linux (`PackageKind::preferred`, the same choice it makes for a craft). The MSI
and the deb/rpm are for people who prefer an installer or their package manager; a copy
installed that way doesn't update itself (Program Files and `/usr/bin` aren't writable without
elevation, and the toolbox never elevates): it says so and is updated the way it was installed.

Every binary reports its version, the commit and the build date: `artcraft-toolbox --version`,
`artcraft-toolbox-cli --version`, and Settings › About. CI sets `ARTCRAFT_TOOLBOX_BUILD_SHA` and
`ARTCRAFT_TOOLBOX_BUILD_DATE`, and `crates/engine/src/build_info.rs` reads them at compile time.
A plain `cargo build` doesn't set them and reports `0.1.0 (dev build)`.

### macOS

`packaging/macos/package.sh` builds `aarch64-apple-darwin` and `x86_64-apple-darwin` with
`MACOSX_DEPLOYMENT_TARGET=11.0`, joins them with `lipo`, and assembles `ArtCraft Toolbox.app`:

- `Info.plist` is generated from `Info.plist.in`. The bundle id is `ai.storyteller.toolbox`, the
  executable and icon are named `ArtCraft Toolbox`. `LSUIElement` is set: a menu-bar app with
  no Dock icon (the app sets the accessory activation policy at run time too). The icon is
  `assets/app-icon/artcraft-toolbox.icns`. No document types: the toolbox opens no files.
- **Signing** goes inside-out with the hardened runtime and a secure timestamp. The executable
  is signed first, then the bundle. There's no `--deep` on the final signature. The
  entitlements (`entitlements.plist`) are deliberately empty. The script verifies the bundle
  with `codesign --verify --strict --deep`: the toolbox's own bundle must pass the stricter
  check the crafts' don't (release contract › Gotchas 8).
- **Notarization:** the app is zipped and sent with `xcrun notarytool submit --wait`, then the
  ticket is stapled to the app. The app goes on a DMG (`hdiutil`, with an `Applications` link
  to drag onto). The DMG is signed, notarized and stapled too. Its Finder window (background,
  icon size and positions) comes from [`packaging/macos/dmg/`](../packaging/macos/dmg/README.md),
  and its volume is named `ArtCraft Toolbox` without the version, which the window's background
  needs; the DMG file name keeps the version.
- **CLI:** the universal `artcraft-toolbox-cli` is signed with the same Developer ID, the
  hardened runtime and a secure timestamp (identifier `ai.storyteller.toolbox-cli`), zipped,
  and the zip is sent to `notarytool`. Only `.app`, `.dmg` and `.pkg` can hold a stapled
  ticket, not a bare Mach-O, so Gatekeeper looks the CLI's ticket up online the first time a
  downloaded (quarantined) copy runs.
- **Verification:** `packaging/macos/verify.sh` checks the artifacts as users download them,
  and as the toolbox itself checks a craft's DMG: it attaches the image at a private mount
  point, requires exactly one `.app` with the toolbox's bundle id, and verifies its signature
  (`--deep` without `--strict`: `hdiutil makehybrid` gives the image's files HFS type and
  creator codes, `com.apple.FinderInfo` on the `.icns` and `.txt` files, which `--strict`
  rejects as "detritus" although the signature is intact; the root cause of the crafts'
  release contract › Gotchas 8, measured here on 2026-10-10: 12 files in the toolbox's own
  image, none in the folder it was made from);
  for the CLI it requires `codesign --verify --strict`, both architectures, the hardened
  runtime flag, a `Developer ID Application` authority from `APPLE_TEAM_ID`, a timestamp, and
  `spctl --assess --type install` reporting `source=Notarized Developer ID`. The release
  workflow runs it as its own step after packaging. With signing secrets it fails the job on
  any miss; without them it only checks signature integrity and warns, like `package.sh`.

Locally, without certificates, the script signs ad-hoc (`codesign -s -`) and skips notarization.
That's enough to check the bundle and the DMG on your own Mac (done on 2026-10-10: the DMG
attaches, holds one `ArtCraft Toolbox.app` with the right bundle id, both binaries print their
version, `verify.sh` passes):

```sh
packaging/macos/package.sh                    # universal; needs both rustup targets
packaging/macos/package.sh --arch aarch64     # quicker, host-only
packaging/macos/verify.sh --arch aarch64
open dist/release/artcraft-toolbox-*-macos-*.dmg
```

### Windows

`packaging/windows/package.ps1 -Arch x64|x86|arm64` builds with `-C target-feature=+crt-static`.
The static C runtime means neither the MSI nor the portable zip needs the Visual C++
redistributable. The flag goes in `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`, so host build scripts
aren't affected.

- `apps/artcraft-toolbox/build.rs` and `apps/artcraft-toolbox-cli/build.rs` embed the icon
  (`assets/app-icon/artcraft-toolbox.ico`) and VERSIONINFO with the `winresource` crate. They
  only do this when targeting Windows; elsewhere each script is a no-op. A missing resource
  compiler warns, so an optional cross-compile still links; `package.ps1` sets
  `ARTCRAFT_TOOLBOX_REQUIRE_WINRES=1`, which turns that into a build error.
- Release builds use the GUI subsystem, so Start Menu launches don't open a console window.
  The CLI stays console subsystem 3 so its output reaches the terminal. The script checks both
  PE headers (machine and subsystem) before packaging.
- `artcraft-toolbox.wxs` (WiX v5) is a per-machine install into Program Files with an
  advertised Start Menu shortcut and App Paths (Win+R `artcraft-toolbox`). No file
  associations. The MSI version is the numeric `X.Y.Z`, because MSI has no pre-release field.
  Same-version upgrades are allowed so that release candidates replace each other.
- Double-clicking the MSI opens a setup wizard (WixUI_InstallDir without the licence page):
  Welcome, install folder (remembered for upgrades in `HKLM\Software\ArtCraft Toolbox\InstallDir`),
  Ready, a progress page, and a Finish page with a ticked "Launch ArtCraft Toolbox" box.
  `package.ps1` draws the wizard's banner and side bitmaps from the app icon, so no WiX stock
  art ships. `msiexec /qn` still installs silently.
- Shortcut icon identifiers keep the executable's `.exe` extension (ICE50);
  `packaging/windows/check-icons.ps1` checks the references in CI and `package.ps1` validates
  the built MSI with ICE50 before signing it.
- **Portable zip:** `artcraft-toolbox.exe` and `artcraft-toolbox-cli.exe` with the licences.
  Unlike the crafts' zips it ships **no `portable.txt`**: the toolbox has no portable mode (its
  data lives in `%APPDATA%\ArtCraft Toolbox`), the folder can be moved freely, and the toolbox
  updates itself in place there (the running exe is renamed to `artcraft-toolbox.exe.previous`
  and the new one copied in; the leftover is removed at the next start).
- **Signing:** `packaging/windows/sign.ps1` signs both `.exe` files and then the `.msi` with
  `signtool`, using SHA-256 and an RFC 3161 timestamp, with whichever material is present: a
  `.pfx` (`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`) or Azure Trusted Signing
  (`AZURE_*`). The script is the single place to change when the Windows signing setup changes.

Locally on Windows: `dotnet tool install -g wix --version 5.0.2`,
`wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2`, then
`pwsh packaging/windows/package.ps1 -Arch x64`.

### Linux

`packaging/linux/package.sh` stages one FHS tree and makes every format from it. The tree
holds both binaries, `ai.storyteller.toolbox.desktop`, hicolor icons from 16 px to 512 px plus
the scalable SVG, and AppStream metainfo.

- **AppImage** runs on any distribution without installing anything, and is what the toolbox
  installs for itself. Built with the maintained `AppImage/appimagetool`, whose static runtime
  doesn't need libfuse2. Each AppImage embeds update information
  (`gh-releases-zsync|…|latest|…`) and ships with a `.zsync` beside it. Its `AppRun`
  (`packaging/linux/AppRun`) integrates with the desktop on launch: it installs
  `~/.local/share/applications/ai.storyteller.toolbox.desktop` with `Exec` rewritten to the
  AppImage's path and the hicolor icons beside it, rewriting only when the content changed, so
  Wayland docks show the toolbox's icon rather than the generic one. Any failure leaves the
  launch untouched. `ARTCRAFT_TOOLBOX_NO_DESKTOP_INTEGRATION=1` opts out. The script is
  covered by `packaging/linux/apprun-test.sh` in the packaging-lint workflow.
- **.deb** and **.rpm** are built by [nfpm](https://nfpm.goreleaser.com/) from one config
  (`nfpm.yaml`), integrate with the menu and icon caches (`postinst.sh`) and uninstall cleanly.
  A copy installed this way is updated by the package manager, not by the toolbox.
- **.tar.gz** is for people who manage their own `/opt` or `~/.local`.

All Linux binaries are built on Ubuntu 22.04 and need **glibc ≥ 2.35**. They link only glibc and
libgcc_s; X11, Wayland, xkbcommon, Vulkan and EGL are loaded at run time from the system (the
full list, and why each is there, is in `packaging/linux/nfpm.yaml`), and the tray icon speaks
StatusNotifierItem over D-Bus, so no GTK is needed.

Locally (on Linux): install [nfpm](https://nfpm.goreleaser.com/install/), then
`packaging/linux/package.sh` (or `--formats "deb tar"`).

## Secrets

All secrets live in the repository's **`release` environment** (*Settings → Environments →
release*). Restrict its deployment branches to `release`. Every job in `release.yml` that
signs or publishes declares `environment: release` (the Linux job signs nothing and doesn't),
so only pushes to that branch, or manual runs on it, can read the secrets. Each secret is
optional. If one is missing, that platform's artifacts are unsigned and the run shows a
`::warning::`.

| Secret | Used for |
|---|---|
| `APPLE_CERTIFICATE` | base64 `.p12` with the "Developer ID Application" certificate |
| `APPLE_CERTIFICATE_PASSWORD` | password for that `.p12` |
| `KEYCHAIN_PASSWORD` | password for the temporary CI keychain (random if unset) |
| `APPLE_ID` | Apple ID used by `notarytool` |
| `APPLE_PASSWORD` | app-specific password for that Apple ID |
| `APPLE_TEAM_ID` | the team id |
| `WINDOWS_CERTIFICATE` | base64 `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | password for that `.pfx` |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` | service principal for Azure Trusted Signing (an alternative to the `.pfx`) |
| `AZURE_SIGNING_ENDPOINT` | e.g. `https://eus.codesigning.azure.net` |
| `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Trusted Signing account and certificate profile names |

`GITHUB_TOKEN` creates the release. Only the final job gets `contents: write`.

On macOS, `packaging/macos/import-cert.sh` decodes the `.p12` into a temporary keychain and
exports the identity's SHA-1 as `MACOS_SIGN_IDENTITY`. The keychain is deleted at the end of
the job.

**The signer is part of the contract with the toolbox's users:** once a signed version is
installed, the toolbox refuses any later version signed by anyone else. Changing the
certificate's team (or shipping an unsigned release after signed ones) means every user has to
reinstall by hand. No signing secrets are configured yet (2026-10-10): the first releases are
ad-hoc signed on macOS and unsigned on Windows, which the toolbox shows as "No platform
signature" and accepts as the reference for later ones.

## Icons

`assets/app-icon/artcraft-toolbox.svg` is the canonical icon (`assets/app-icon/README.md`): an
engraved toolbox in the crafts' Ink-and-Paper style on a steel field.
`packaging/icons.sh` regenerates the 1024 px PNG, the `.icns`, the `.ico` (packed by
`cargo xtask ico`), the hicolor PNGs, the window icon and the tray icons from it. It needs
`resvg`, plus `iconutil` on macOS. The outputs are committed, so packaging never needs those
tools, and the packaging-lint workflow checks they exist and that the scalable icon is the
master.

## Checks

`.github/workflows/packaging-lint.yml` runs in seconds on any change to `packaging/`, the
workflows or the icons: actionlint, shellcheck, the AppRun smoke test, a PowerShell parse and
the MSI icon check, xmllint, `desktop-file-validate` and `appstreamcli validate`, and the icon
renders. `cargo xtask contract --dir <folder>` checks a folder of built artifacts against the
release contract offline; `cargo xtask contract --app artcraft-toolbox` checks the published
releases, like a craft's.
