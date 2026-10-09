# The release contract

Everything ArtCraft Toolbox relies on from the Crafting Apps. Every craft builds its releases with
the same pipeline (PhotoCraft's `.github/workflows/release.yml` and `packaging/`, from the shared
playbook in `../craftrules/release/playbook.md`), so one parser serves all of them.

`cargo xtask contract` checks the newest stable release of every catalog app against this page
using the toolbox's own parsers; `.github/workflows/contract.yml` runs it every day. If it fails,
a craft changed how it publishes: update this page, the parser and a fixture in one change.

## Where

- GitHub Releases of `github.com/<repo>` (`repo` in `crates/catalog/catalog.toml`; today
  `storytold/<id>` for every app).
- Listed by `GET https://api.github.com/repos/<repo>/releases` (newest first).
- Downloads are each asset's `browser_download_url`, `https://github.com/<repo>/releases/download/<tag>/<file>`,
  which redirects to GitHub's asset CDN. The toolbox follows that redirect and no other
  (`feed::github::DOWNLOAD_PREFIX`).

## Releases

- Tag `v<version>`; version is `MAJOR.MINOR.PATCH` with an optional pre-release
  (`v1.0.0-rc.1`). Title `<Name> v<version>`.
- Drafts are invisible to the API without a token and are skipped anyway.
- Pre-releases: a `-suffix` version is marked as a GitHub pre-release. The toolbox treats either
  signal as a pre-release (`Release::is_prerelease`), offered only on the Pre-release channel.
- Notes are generated from merged PRs (Markdown).

## Assets

`<id>-<version>-<os>-<arch>.<ext>`, parsed right to left so pre-release hyphens are safe
(`release::asset::parse`):

| Asset | os-arch | Toolbox use |
|---|---|---|
| `<id>-<v>-macos-universal.dmg` | Apple silicon + Intel | **installed on macOS** |
| `<id>-<v>-windows-{x64,x86,arm64}-portable.zip` | Windows 10+ | **installed on Windows** |
| `<id>-<v>-windows-{x64,x86,arm64}.msi` | Windows 10+ | fallback when no portable zip |
| `<id>-<v>-linux-{x86_64,aarch64}.AppImage` | glibc ≥ 2.35 | **installed on Linux** |
| `<id>-<v>-linux-{x86_64,aarch64}.AppImage.zsync` | | delta updates (not used yet) |
| `<id>-<v>-linux-{x86_64,aarch64}.{deb,rpm,flatpak}` | | not used (system package managers own those) |
| `<id>-<v>-linux-{x86_64,aarch64}.tar.gz` | | fallback when no AppImage |
| `<id>-<v>-freebsd-x86_64.tar.gz` | FreeBSD 14 | installed on FreeBSD |
| `<id>-cli-<v>-macos-universal.zip` | | not used (the CLI ships inside the other packages) |
| `<id>-web-<v>.zip` | static site | not used |
| `SHA256SUMS.txt` | | **required**: verifies every download |

Windows architecture tokens are `x64`/`x86`/`arm64`; Linux and FreeBSD use `x86_64`/`aarch64`.
`release::Arch::from_token` accepts both.

## Integrity

`SHA256SUMS.txt` is GNU `sha256sum` output for every other asset, written by the same release job
(`release` job in the craft's `release.yml`). It proves a download is intact and matches the
release; it does not prove who made the release. Platform signatures add that where they exist:

- macOS: the app and DMG are Developer ID signed, notarized and stapled when the craft's release
  secrets are configured (PhotoCraft 0.2.0 shipped notarized).
- Windows: Authenticode signing is optional in the pipeline and skipped with a warning when the
  signing material is missing (PhotoCraft's scorecard, DIST-2: partial). The toolbox can't require
  it yet; it should report unsigned builds.

## Measured state (2026-10-09)

`cargo xtask contract`: all 12 catalog apps pass (22 assets per latest release, every host
installable, every asset listed in `SHA256SUMS.txt`).

| App | Latest | | App | Latest |
|---|---|---|---|---|
| PhotoCraft | v0.5.0 | | CADCraft | v0.3.0 |
| VectorCraft | v0.7.0 | | DeckCraft | v0.3.0 |
| FilmCraft | v0.4.0 | | GridCraft | v0.3.0 |
| LightCraft | v0.4.0 | | SoundCraft | v0.3.0 |
| PdfCraft | v0.4.0 | | WordCraft | v0.3.0 |
| EffectCraft | v0.6.0 | | | |
| DesignCraft | v0.4.0 | | | |

## Gotchas

1. **Renames.** PdfCraft published 0.1.x–0.2.x as PrintCraft (`printcraft-*` assets in the
   `pdfcraft` repo). `former_slugs = ["printcraft"]` in the catalog keeps those releases readable.
2. **Older releases have fewer builds.** Before ~0.3/0.4, releases had no Windows arm64, Flatpak,
   zsync or FreeBSD builds. Windows on ARM then falls back to the x64 build under emulation.
3. **The Windows portable zip carries `portable.txt`.** It switches the app to portable mode:
   preferences, presets and autosaves go to `<Name>Data` beside the exe instead of
   `%APPDATA%\<Name>`. With one directory per version, the user's data would stay behind in the
   old version's directory. The toolbox must delete `portable.txt` after extracting (decided in M2).
4. **Some crafts check for updates themselves.** PdfCraft has an in-app check ("check only when
   asked", 0.2.1 notes); PhotoCraft has none (2026-10-09). If a craft ever updates itself in place,
   the inventory drifts, so the toolbox re-reads the installed version on refresh (`Info.plist`
   `CFBundleShortVersionString`, or `<exe> --version`: PhotoCraft has it; check each craft in M3).
5. **AppImages carry update information** (`gh-releases-zsync|storytold|<id>|latest|…`) and a
   `.zsync` file sits beside each: delta updates are possible later.
6. **ArtCraft itself does not follow the contract.** `storytold/artcraft` (the AI studio) is a
   different stack: tags `artcraft-v0.41.0`, assets `ArtCraft_0.41.0_universal.dmg`,
   `ArtCraft_0.41.0_x64-setup.exe`, `ArtCraft_0.41.0_x64_en-US.msi`, no Linux build and no
   checksums. It is not in the catalog; supporting it is roadmap M7.
7. **The DMG volume is named after the product without the version** and holds `<Name>.app` plus
   an `Applications` link.

## Refreshing the fixtures

`crates/feed/tests/fixtures/*.json` are real responses, trimmed (four releases, notes replaced so
no contributor data is stored):

```sh
gh api 'repos/storytold/photocraft/releases?per_page=4' > /tmp/raw.json
# keep tag_name, name, draft, prerelease, published_at, html_url and assets[name, size,
# browser_download_url]; set every body to "Release notes trimmed for this test fixture."
```
