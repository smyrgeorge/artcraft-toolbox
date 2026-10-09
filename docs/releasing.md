# Releasing ArtCraft Toolbox

Not set up yet: it is roadmap milestone M5.

The plan is to reuse the Crafting Apps' release pipeline unchanged in shape, so the toolbox
follows its own release contract (`docs/release-contract.md`) and can update itself:

1. Port PhotoCraft's `.github/workflows/release.yml` and `packaging/` (macOS universal DMG signed
   and notarized; Windows MSI and portable zip for x64, x86 and arm64; Linux AppImage, deb, rpm and
   tar.gz for x86_64 and aarch64), renaming `photocraft` to `artcraft-toolbox` and dropping what the
   toolbox doesn't need (fonts, HEIF, file associations, the web and FreeBSD jobs unless wanted).
   The canonical recipe is `../craftrules/release/playbook.md`; PhotoCraft's
   `docs/releasing.md` explains its specifics.
2. Every push to the `release` branch builds signed artifacts and a draft GitHub Release
   `ArtCraft Toolbox v<version>` with `SHA256SUMS.txt`; a maintainer publishes it.
3. The version lives only in `[workspace.package] version`: `cargo xtask version set X.Y.Z`
   (already available).
