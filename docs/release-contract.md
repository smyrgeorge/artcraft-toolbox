# The release contract

Everything ArtCraft Toolbox relies on from the Crafting Apps. Every craft builds its releases with
the same pipeline (PhotoCraft's `.github/workflows/release.yml` and `packaging/`, from the shared
playbook in `../craftrules/release/playbook.md`), so one parser serves all of them.

`cargo xtask contract` checks the newest stable release of every catalog app against this page
using the toolbox's own parsers; `.github/workflows/contract.yml` runs it every day. If it fails,
a craft changed how it publishes: update this page, the parser and a fixture in one change.

**The toolbox itself follows this contract** (`docs/releasing.md`): its releases are
`artcraft-toolbox-<version>-<os>-<arch>.<ext>` with `SHA256SUMS.txt`, tagged `v<version>`, at the
`[toolbox]` repo of `catalog.toml`, minus the Flatpak, FreeBSD and web assets. That is how it
updates itself (docs/architecture.md § 7). `cargo xtask contract --app artcraft-toolbox` checks
its published releases, `--dir <folder>` a folder of freshly built ones (the release workflow
runs it before uploading).

## Where

- GitHub Releases of `github.com/<repo>` (`repo` in `crates/catalog/catalog.toml`; today
  `storytold/<id>` for every app).
- Listed by `GET https://api.github.com/repos/<repo>/releases` (newest first).
- Downloads are each asset's `browser_download_url`, `https://github.com/<repo>/releases/download/<tag>/<file>`
  (`feed::github::DOWNLOAD_PREFIX`), which answers `302` to GitHub's asset CDN,
  `release-assets.githubusercontent.com` (formerly `objects.githubusercontent.com`). The toolbox
  follows redirects to those hosts and no others (`net::Policy::github`, checked per hop), never
  sends a token there, and resumes an interrupted download with `Range: bytes=<n>-`: the CDN
  answers `206` with a `Content-Range` that is checked (measured 2026-10-09).

## GitHub API (measured 2026-10-09)

- The toolbox asks for `?per_page=20`: enough to find the newest stable build behind a run of
  pre-releases. Responses are large because release notes are inlined: PhotoCraft's 30 newest
  releases were 304 KB uncompressed (gzip is negotiated); cached feeds are 45–320 KB per app.
- **Anonymous requests: 60 per hour per IP address, and a `304 Not Modified` still counts.**
  Measured: three conditional requests in a row, all answered `304`, took
  `x-ratelimit-remaining` from 47 to 44. ETags save bandwidth, not quota. One full check of 13
  apps and the toolbox costs 14 requests, so an anonymous user gets about four full checks an
  hour from the API. Hence the toolbox's back-off (`engine::update_cmds`) and, since M7, the
  publisher's aggregated feed: one request to `raw.githubusercontent.com` (no such limit) per
  check, the API only for what the feed doesn't cover (§ Apps outside the contract, and
  docs/architecture.md § 12).
- With a token (`ARTCRAFT_TOOLBOX_GITHUB_TOKEN`) the limit is 5,000 per hour. Observed: the first
  authenticated request with an ETag from an anonymous response got a full `200`, not a `304`.
- A refusal is `403` with `x-ratelimit-remaining: 0` and `x-ratelimit-reset` (Unix seconds), or
  `429`/`403` with `retry-after` for secondary limits; `net::classify` turns either into
  `NetError::RateLimited`.
- Public release lists need no credentials, so a `401` means the user's token is bad; the toolbox
  says so.

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

## Icons

Every craft keeps its icon at the same place in its repository (PhotoCraft's `packaging/icons.sh`
writes them): `assets/app-icon/hicolor/<n>x<n>/apps/ai.storyteller.<id>.png`, n = 16 … 512. The
toolbox reads the 128 px one from
`https://raw.githubusercontent.com/<repo>/HEAD/assets/app-icon/hicolor/128x128/apps/<bundle id>.png`
(`App::icon_url`; a catalog `icon` field overrides it). `raw.githubusercontent.com` is not the
rate-limited API and never receives a token. Checked 2026-10-09: all 12 apps serve both the 64 and
128 px icons there.

## Integrity

`SHA256SUMS.txt` is GNU `sha256sum` output for every other asset, written by the same release job
(`release` job in the craft's `release.yml`). It proves a download is intact and matches the
release; it does not prove who made the release. Platform signatures add that where they exist:

- macOS: the app and DMG are Developer ID signed, notarized and stapled when the craft's release
  secrets are configured (PhotoCraft 0.2.0 shipped notarized).
- Windows: Authenticode signing is optional in the pipeline and skipped with a warning when the
  signing material is missing (PhotoCraft's scorecard, DIST-2: partial). The toolbox can't require
  it; it reports unsigned builds.

What the toolbox does with them (M3, `install::trust`): a broken signature is refused, a missing
one is shown ("No platform signature"), and the developer that signed the installed version is
recorded: an update signed by anyone else is refused. Measured 2026-10-09: PhotoCraft 0.3.0 and
0.5.0 are both "Developer ID Application: Learning Machines LLC (DJ6XS33FX8)", notarized
(`spctl`: "source=Notarized Developer ID").

## Managed apps

The toolbox opens a craft with `ARTCRAFT_TOOLBOX_MANAGED=1` in its environment (on macOS through
`open --env`). A craft that checks for updates itself can leave that to the toolbox when it sees
it: the toolbox keeps versions for rollback and checks signatures, and an in-place self-update
makes its inventory drift. The toolbox never passes its own `ARTCRAFT_TOOLBOX_*` variables (a
GitHub token among them) to an app. Started any other way (the Dock, the Start Menu), a craft gets
no signal; the toolbox notices a self-update on its next look at the disk (Gotchas 4).

## Measured state (2026-10-10)

`cargo xtask contract`: all 12 catalog apps pass (22 assets per latest release, 24 for PdfCraft,
every host installable, every asset listed in `SHA256SUMS.txt`). Every craft released again on
2026-10-10; the toolbox's own cached feeds followed without a code change.

| App | Latest | | App | Latest |
|---|---|---|---|---|
| PhotoCraft | v0.6.0 | | CADCraft | v0.4.0 |
| VectorCraft | v0.8.0 | | DeckCraft | v0.4.0 |
| FilmCraft | v0.5.0 | | GridCraft | v0.4.0 |
| LightCraft | v0.5.0 | | SoundCraft | v0.4.0 |
| PdfCraft | v0.5.0 | | WordCraft | v0.4.0 |
| EffectCraft | v0.7.0 | | | |
| DesignCraft | v0.5.0 | | | |

## Gotchas

1. **Renames.** PdfCraft published 0.1.x–0.2.x as PrintCraft (`printcraft-*` assets in the
   `pdfcraft` repo). `former_slugs = ["printcraft"]` in the catalog keeps those releases readable.
2. **Older releases have fewer builds.** Before ~0.3/0.4, releases had no Windows arm64, Flatpak,
   zsync or FreeBSD builds. Windows on ARM then falls back to the x64 build under emulation.
3. **The Windows portable zip carries `portable.txt`.** It switches the app to portable mode:
   preferences, presets and autosaves go to `<Name>Data` beside the exe instead of
   `%APPDATA%\<Name>`. With one directory per version, the user's data would stay behind in the
   old version's directory. The toolbox deletes `portable.txt` (and `<Name>.portable`) after
   extracting (M2).
4. **Some crafts check for updates themselves.** PdfCraft has an in-app check ("check only when
   asked", 0.2.1 notes); PhotoCraft has none (2026-10-09). If a craft ever updates itself in place,
   the inventory drifts, so the toolbox re-reads the installed version when it looks at the disk
   (`apps.rescan`, and at start): on macOS from `Info.plist` `CFBundleShortVersionString`
   (verified by replacing an installed PhotoCraft 0.3.0 with 0.5.0). On Windows and Linux the
   version is the folder's name; a craft updating itself there would have to write a new folder.
5. **AppImages carry update information** (`gh-releases-zsync|storytold|<id>|latest|…`) and a
   `.zsync` file sits beside each: delta updates are possible later.
6. **ArtCraft itself does not follow the contract.** `storytold/artcraft` (the AI studio) is a
   different stack: tags `artcraft-v0.41.0`, assets `ArtCraft_0.41.0_universal.dmg`,
   `ArtCraft_0.41.0_x64-setup.exe`, `ArtCraft_0.41.0_x64_en-US.msi`, no Linux build and no
   checksums. Since M7 it is in the catalog through `[app.assets]` patterns, and its digests
   come from the signed feed (§ Apps outside the contract). Measured 2026-10-10: bundle id
   `ai.artcraft.app`, signed and notarized by the same team as the crafts (DJ6XS33FX8); the
   Windows build is an MSI only, which the toolbox doesn't install, so no Windows pattern.
7. **The DMG volume is named after the product without the version** and holds `<Name>.app` plus
   an `Applications` link. The toolbox mounts it at a private mount point, so two installs never
   collide on `/Volumes/<Name>`, and requires exactly one `.app` whose `CFBundleIdentifier` is
   `ai.storyteller.<id>`.
8. **The apps in the DMGs carry Finder information on some files** (`com.apple.FinderInfo` on 15
   files of PhotoCraft 0.3.0 and 0.5.0, inside the image already). `codesign --verify --strict`
   rejects that as "detritus" although the signature is intact and Gatekeeper accepts the app, so
   the toolbox verifies without `--strict` (a changed file or executable still fails). The cause
   (found 2026-10-10 on the toolbox's own DMG, built the same way): `hdiutil makehybrid -hfs`
   gives files HFS type and creator codes by extension (`.icns`, `.txt`, …), stored as that
   xattr; the staged folder has none. `makehybrid` has no option to turn that off, so fixing it
   means another DMG builder; until then `--strict` can't be used on any craft's bundle.

## Apps outside the contract

An app that doesn't publish the asset set above can still be listed, with its builds named per
target in the catalog (`crates/catalog/catalog.toml` › `[app.assets]`):

```toml
[[app]]
id = "artcraft"
name = "ArtCraft"
repo = "storytold/artcraft"
bundle_id = "ai.artcraft.app"
icon = "https://raw.githubusercontent.com/storytold/artcraft/HEAD/crates/desktop/artcraft/icons/128x128.png"

[app.assets]
"macos-universal" = "ArtCraft_{version}_universal.dmg"
```

- The key is `<os>-<arch>` as in asset names; the value is the whole file name with `{version}`
  where the version goes. `release::asset::from_pattern` matches it; the package kind comes from
  the extension, so only kinds the toolbox installs (DMG, portable zip, AppImage) are useful.
- The tag may be `v<version>`, `<version>` or `<name>-v<version>` (`Version::from_tag`).
- Such a release has no `SHA256SUMS.txt`. **The digests come from the publisher's signed
  aggregated feed**: `cargo xtask feed` downloads each matched build once, hashes it and writes
  `sha256` on the asset (the toolbox reads it from the signed document only). A session that
  got the app's feed from the GitHub API instead has no digest, and `app.install` refuses:
  "the toolbox only installs what it can verify".
- The platform signature is checked as for any app (`install::trust`), and the first install
  pins the signer for later updates.
- `cargo xtask contract` checks that every pattern matches a build of the newest stable release.

## The aggregated feed and the remote catalog

Published by the Feed workflow (`.github/workflows/feed.yml`, hourly) on this repository's
`feed` branch and served by `raw.githubusercontent.com`:

| Document | Payload | Use |
|---|---|---|
| `artcraft-catalog.json` | `catalog.toml` (schema 1, with its `revision`) | New apps, moved repos or URLs reach every toolbox without a release; applied when the revision is not older than the running one |
| `artcraft-feed.json` | `{"schema":1,"generated":<unix>,"apps":{"<id>":[<GitHub release objects, trimmed>]}}` | Every app's releases in one request; each list is read by the same parser as a direct API response and cached per app |

Both are `release::signing` envelopes: `{"schema":1,"kind":"catalog|feed","signer":"ed25519:…",
"signature":"<base64>","payload":"<text>"}`, Ed25519 over `"artcraft-toolbox <kind> v1\n"` +
payload. The toolbox pins the public key compiled into its built-in catalog (`[remote]`); a
remote catalog may move the URLs, never the key. Rejected documents (bad signature, other key,
older revision, wrong kind, unreadable) are logged and the check goes on: the built-in catalog
and the per-app API requests are the fallback.

## Refreshing the fixtures

`crates/feed/tests/fixtures/*.json` are real responses, trimmed (four releases for the crafts,
twenty for ArtCraft because its tags and duplicate versions are what the patterns are tested
against; notes replaced so no contributor data is stored):

```sh
gh api 'repos/storytold/photocraft/releases?per_page=4' > /tmp/raw.json
# keep tag_name, name, draft, prerelease, published_at, html_url and assets[name, size,
# browser_download_url]; set every body to "Release notes trimmed for this test fixture."
```
