# Roadmap

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Updated 2026-10-10.

| M | Status | Goal |
|---|---|---|
| M0 Skeleton | ✅ | PhotoCraft's workspace, conventions and CI; the pure core; engine, CLI and UI shell |
| M1 Live feeds | ✅ | Fetch, cache and show every app's releases; persist settings and inventory |
| M2 Install and launch | ✅ | Install, uninstall and open any craft on macOS, Windows and Linux |
| M3 Updates, rollback, trust | ✅ | Update one or all, keep and switch versions, verify signatures, adopt existing installs |
| M4 Toolbox UX parity | ✅ | Menu-bar/tray app, background checks, notifications, release notes, per-app settings |
| M5 Distribution and self-update | ✅ | Signed installers via PhotoCraft's release pipeline; the toolbox updates itself |
| M6 Automation | ✅ | Control channel and MCP over the command registry |
| M7 Catalog from the network | ✅ | A signed remote catalog; non-conforming apps (ArtCraft itself) |
| M8 Polish | ✅ | Localization (PhotoCraft's `tl!` pattern), accessibility, light theme |

## Current focus: every milestone has landed; next is shipping

v0.1.0 was published on 2026-10-10 (unsigned: no certificates yet) and the signed feed is live
(the `feed` branch, refreshed hourly). Open work, in order: configure the signing secrets
(docs/releasing.md) and cut the first signed release; then the dev log's "still open" bullets
(`toolbox.rollback`, Windows and Linux runs on real machines) and the open questions below.

Agents: pick the first unchecked item of the current milestone. Tick it in the same change that
lands it.

### M0 Skeleton ✅ (2026-10-09)

- [x] Workspace mirroring PhotoCraft: edition 2024, rust-version 1.95, lints, profiles, rustfmt, clippy.toml, `cargo xtask` alias
- [x] Layering enforced (`cargo xtask layers`), including "no I/O crates below L3"
- [x] `release`: versions (semver precedence), asset names, targets, SHA256SUMS; hostile-input tests
- [x] `catalog`: the 12 Crafting Apps as data, validated
- [x] `model`: inventory (side-by-side versions, active one) and settings
- [x] `feed`: GitHub releases → typed releases; update status per app; real captured fixtures
- [x] `engine`: Session, command registry, `catalog.list`, `app.info`, `app.status`, `apps.status`, `settings.get`, `settings.set`; panic hunt over every command
- [x] CLI: `list`, `status --feed`, `commands`, `run`
- [x] Desktop app and egui shell: app list (installed / available), search, settings; offscreen snapshot example; kittest UI tests
- [x] `cargo xtask contract`: every craft's live release checked against the contract (12/12 pass)
- [x] CI (`ci.yml`), daily contract check (`contract.yml`), AGENTS.md, docs

### M1 Live feeds ✅ (2026-10-09)

- [x] Data directory (`store::dirs`, ported from PhotoCraft's `app_dirs.rs`: `ARTCRAFT_TOOLBOX_CONFIG_DIR`, else the platform default; no portable mode)
- [x] `settings.json` and `inventory.json` saved atomically (`store::atomic`, ported from PhotoCraft); unreadable settings are backed up and replaced by defaults, an unreadable inventory is reported, locked and never overwritten
- [x] File logging with rotation (PhotoCraft's `logging.rs`, ported): `<data dir>/logs/artcraft-toolbox.log`
- [x] `net` crate (L3): ureq + rustls with the OS trust store; GitHub-only host policy checked on every redirect hop; token only to `api.github.com`; body caps, timeouts, rate-limit headers (docs/architecture.md § 8 records the choice)
- [x] Feed cache in the data dir (raw body + ETag); conditional requests; optional token in `ARTCRAFT_TOOLBOX_GITHUB_TOKEN`
- [x] `updates.check` (all apps or one) as a background job (`Session::start` / `poll_jobs` / `wait_job` / `cancel_job`); stops at GitHub's rate limit and stays disabled until its reset; apps checked in the last minute aren't requested again
- [x] UI: "Check for updates" with progress, per-row spinners, last-checked time and errors in the status bar
- [x] Check on start when a feed is older than `checkIntervalHours`
- [x] CLI: `check [--app <id>] [--force] [--json]`; `status` reads the cache; usage errors touch nothing

Measured along the way: anonymous `304`s still count against GitHub's 60 requests/hour (see
`docs/release-contract.md` › GitHub API), so one full check costs 12 of them. That shapes M7.

### M2 Install and launch ✅ (2026-10-09)

- [x] `install` crate (L3): one `install` / `uninstall` entry dispatching on the package kind, a per-OS `Layout`; tests against temp dirs (a real `hdiutil` round trip on macOS)
- [x] Download to `downloads/<file>.part` with progress and HTTP range resume (`Transport::download`: GitHub's asset CDN allowed, the token never sent); `SHA256SUMS.txt` fetched first, exact size and SHA-256 checked before the package is opened
- [x] macOS: mount the DMG read-only, check the bundle id, copy `<Name>.app` into `~/Applications` with `ditto`, detach
- [x] Windows: extract the portable zip safely (zip-slip, drive paths, symlinks, duplicates, size caps), delete `portable.txt`, Start Menu shortcut
- [x] Linux: check and place the AppImage, `chmod +x`, desktop entry (marked as the toolbox's) and icon under `~/.local/share`
- [x] `app.install` (background job, cancellable), `app.uninstall`, `app.launch`; Install / Open / Cancel live in the list, Uninstall (with a confirmation) on the app's page; CLI `install` / `uninstall` / `open`
- [x] Install layout decided and documented (docs/architecture.md § 5)

Verified live on macOS: the CLI installed PhotoCraft 0.5.0 from GitHub (checksum and bundle id
checked), resumed a download interrupted at 41%, and uninstalled it, leaving no mounted image.
Not run on a real machine yet: the Windows Start Menu shortcut and the Linux desktop entry (both
are covered by tests up to the OS call). An app that is already installed can't be installed
again: updating it is M3.

### M3 Updates, rollback, trust ✅ (2026-10-09)

- [x] `app.update`, `apps.updateAll` ("Update all" in the UI when more than one update waits); on macOS never while the app runs (its bundle would move), elsewhere the new version goes in beside the running one
- [x] Keep `keepPrevious` versions (macOS: `versions.noindex/<id>/<version>/`); `app.rollback` switches without a download; `app.versions`; the details page lists versions in use, kept, and installable
- [x] macOS: `codesign --verify --deep` + Gatekeeper (`spctl --assess`) before activating; Windows: Authenticode; a broken signature is refused, and an update signed by another developer than the version it replaces is refused (team id recorded per installation)
- [x] Detect apps installed by hand (`apps.rescan`) and adopt them (`app.adopt`, signature checked); re-read installed versions on rescan (an app that updated itself), forget installs whose files are gone
- [x] Coordinate with crafts that check for updates themselves: launched apps get `ARTCRAFT_TOOLBOX_MANAGED=1` (release contract › Managed apps); honouring it is up to each craft
- [x] Automatic updates (`autoUpdate`, global and per app) after each check, announced when done ("Updated: PhotoCraft 0.5.0") instead of when found
- [x] CLI: `update <app>` / `update --all`, `rollback`, `versions`, `adopt`

Verified live on macOS with real PhotoCraft releases: installed 0.3.0, updated to 0.5.0 (both
"Signed by Learning Machines LLC (DJ6XS33FX8) · notarized"), rolled back in 0.45 s, switched
forward by `update --all` without a download, uninstalled both; a copy installed by hand was
adopted, a tampered one refused ("a sealed resource is missing or invalid"), and an app replaced
in place by a newer version was noticed. Found on the way: `codesign --verify --strict` rejects
genuine PhotoCraft releases (Finder information left on files in the disk image), so the check
runs without `--strict` (release contract › Gotchas 8). Not run on a real machine yet: Windows
Authenticode and the Windows running-app check (CI runs their tests), Linux desktop entries.

### M4 Toolbox UX parity ✅ (2026-10-09, ahead of M2/M3)

- [x] Menu-bar (macOS, template glyph) / tray (Windows, Linux via StatusNotifier, no GTK) icon with Open, Check for Updates, Quit; closing the window keeps the toolbox running (setting `closeToTray`); without a tray host, closing quits
- [x] Periodic background checks while the app runs, also hidden (`eframe::App::logic` ticks every minute; automatic checks back off 15 min after any attempt), and OS notifications for new versions of installed apps, once per version (`state.json`), setting `notifications`
- [x] App details page (click a row): links, status, versions with their release notes rendered from Markdown (bare URLs linked, only web links open)
- [x] Per-app overrides: channel, auto-update, pinned version (`app.settings.get` / `app.settings.set`; the details page; pins cap what is offered, never downgrade)
- [x] App icons from each craft's repository (`raw.githubusercontent.com`, not the API quota), cached with ETags, refreshed weekly (`icons.refresh`); monogram tiles until they arrive
- [x] `app.releases` for agents: versions, notes, installable here, offered by the channel and pin

Still open from M4: a notification click doesn't open the toolbox yet. (The Dock icon went away
with the popover: the toolbox is a macOS accessory app since the 2026-10-09 polish.) Install / Open went live in M2; Update stays disabled until M3.

### M8 Polish ✅ (2026-10-09, ahead of M5–M7)

- [x] Localization on PhotoCraft's pattern (`i18n`, `tl!`, TSV catalogs, plural rules): English plus ja, zh-hans, zh-hant, es, ru, cs, fr, id, ko, pl, de, pt-br, el, it; setting `language` (`auto` follows the system's UI languages, `ARTCRAFT_TOOLBOX_LOCALE` overrides), switched live with the tray menu and notifications (docs/localization.md)
- [x] Chinese, Japanese and Korean from the system's fonts, loaded on demand (PhotoCraft's `cjk` loader; no bundled fonts)
- [x] Light theme (`Tokens::LIGHT`), setting `theme` (system, dark, light); every text colour meets WCAG AA in both themes (tested)
- [x] Text size (setting `textSize`, 90–150 %; Cmd/Ctrl +, −, 0)
- [x] Accessibility: rows are buttons named "PhotoCraft, 0.5.0 available" for screen readers, reachable with Tab and opened with Enter, with a visible focus ring; the progress bar reports its value
- [x] Settings › Appearance: Language (native names), Theme, Text size
- [x] The Studio look (Inter, Lucide icons, PhotoCraft's Studio colours, panels, a menu per installed app, search behind a button, an error banner) and the window as a popover under the menu-bar or tray icon on macOS and Windows (docs/ui-design.md)

Every catalog covers every UI string, plural message, tagline and fixed engine message the UI
shows (tests list what's missing). The CLI, command ids and logs stay English.

### M5 Distribution and self-update ✅ (2026-10-10)

- [x] Port PhotoCraft's `packaging/` and `release.yml` (macOS universal DMG signed and notarized, Windows MSI + portable, Linux AppImage/deb/rpm); `docs/releasing.md`; the `release` job checks the artifacts against the contract (`cargo xtask contract --dir`)
- [x] App icon (replace the placeholder: `assets/app-icon/artcraft-toolbox.svg`, every size from `packaging/icons.sh` and `cargo xtask ico`), Windows resources (`build.rs`), Linux desktop file and AppStream metadata, the DMG's Finder window
- [x] The toolbox follows its own release contract (`[toolbox]` in `catalog.toml`, its feed in every check) and updates itself: `toolbox.status`, `toolbox.update` (download, verify, same signer, stage), swap on next start or `Restart to update` (`toolbox.apply` for the CLI); the previous version kept
- [x] `packaging-lint.yml` as in PhotoCraft

Verified on macOS: `packaging/macos/package.sh --arch aarch64` builds an ad-hoc signed
`ArtCraft Toolbox.app`, its DMG (with the Finder window layout) and the CLI zip, and
`verify.sh` accepts them. The self-update is exercised end to end in tests with a fake GitHub
and an AppImage (engine, UI and CLI). v0.1.0 was built by the pipeline and published on
2026-10-10 (every artifact, `SHA256SUMS.txt`, `cargo xtask contract --app artcraft-toolbox`
passes against it); no signing secrets are configured, so it is ad-hoc signed on macOS and
unsigned on Windows. Still open: a `toolbox.rollback` to the kept previous version; the Windows
rename-and-copy swap and the Linux AppImage exec have only run in tests.

### M6 Automation ✅ (2026-10-10)

- [x] JSON control channel (port PhotoCraft's control-server pattern: localhost, bearer token): `artcraft-toolbox --control <port>` with `--control-token[-file]` or the `ARTCRAFT_TOOLBOX_CONTROL_*` variables; `engine.execute` (waiting for a job or not), `engine.commands`, `jobs.list`, `jobs.cancel`, `app.quit`; the headless `artcraft-toolbox-cli serve` on stdio or a port, with `batch` (docs/control-protocol.md)
- [x] MCP server exposing the command registry: `artcraft-toolbox-cli mcp` (headless) and `mcp --bridge` (the running app); tools `apps_status`, `command_list`, `command_run`, `command_batch`, `jobs_list`, `jobs_cancel`, `ui_get`, `ui_set`, `control_call`; resources `artcraft-toolbox://apps` and `://commands` (docs/mcp.md)
- [x] Every UI state readable and settable (`ui.get`, `ui.set`): the tab, search text and field, the folded panel, the open app page, the uninstall question, the banner; validated as a whole before anything changes

New crate `automation` (L6: tokens, bounded frames, reply budgets, the headless server, the
bridge client, the MCP server on rmcp), `ui-egui/src/control.rs` (the handlers, run between
frames) and `apps/artcraft-toolbox/src/control_server.rs` (the transport). Tested without a
window or a GPU: the handlers (`ui-egui/tests/m6.rs`), the transport, the headless server, the
MCP server over a duplex pipe and over the real line transport, and the bridge against a loopback
control server. Still open: a session with a real MCP client against the packaged binaries; the
app's control server only serves while the process runs (a launcher that wants it must pass
`--control`); no progress notifications for long jobs over MCP (poll `jobs_list`).

### M7 Catalog from the network ✅ (2026-10-10)

- [x] One aggregated, signed release feed for the whole suite (a static file, not the rate-limited API): one request per check instead of 12, and no 60/hour ceiling for anonymous users: `artcraft-feed.json` on the `feed` branch, built hourly by `cargo xtask feed` (`feed.yml`), fetched conditionally from raw.githubusercontent.com at the start of every check; the API only for what it doesn't cover
- [x] Signed remote catalog (new crafts appear without a toolbox release); the built-in catalog stays the fallback: `artcraft-catalog.json`, Ed25519 envelopes (`release::signing`) verified against the key pinned in the built-in catalog, applied when `revision` is not older, cached and applied again at start; setting `remoteCatalog`; `catalog.status`
- [x] Per-app asset patterns for apps outside the contract (ArtCraft's Tauri releases): `[app.assets] "<os>-<arch>" = "Name_{version}.ext"`, digests from the signed feed, `app.install` refuses without one; ArtCraft (`ai.artcraft.app`, macOS) is in the catalog

Tested with a fake publisher and a test key (`engine/tests/remote_catalog.rs`): two requests per
check instead of fourteen, a newer catalog adding apps mid-check and at the next start, tampered,
foreign, older and missing documents falling back to the API with the built-in catalog kept, and
an app outside the contract installing from the feed's digest and refused without one. Live
since 2026-10-10: the secret is set and the Feed workflow published the `feed` branch (14 feeds,
ArtCraft's five newest DMGs hashed); a real check applies the remote catalog, fetches everything
from the aggregated feed and gets 304s on the next one, and the published digest of ArtCraft
0.41.0 matches an independently downloaded DMG. MSI installs (ArtCraft on Windows) stay out of
scope.

## Open questions

- Should Windows default to the MSI (per-user) instead of the portable zip? The zip gives side-by-side versions and no installer UI; the MSI gives Add/Remove Programs and file associations.
- Do crafts' file associations (`.pcraft`, PSD) need registering when the toolbox installs from a portable zip?
- Should the catalog pin each craft's signing team (DJ6XS33FX8 for PhotoCraft) so even the first install is checked against it? Today the first install's signer is trusted and later updates must match it.
- Will the crafts honour `ARTCRAFT_TOOLBOX_MANAGED=1` (PdfCraft's in-app update check), and should the crafts' DMGs drop the Finder information that makes `codesign --strict` fail?
- Publishing: does the toolbox live in `storytold/` with the other crafts? Its name and app id (`ai.storyteller.toolbox`) should be confirmed with the ArtCraft team. Today the `[toolbox]` entry of `catalog.toml` points at `smyrgeorge/artcraft-toolbox` (this repository); moving it is a data change, but every installed toolbox keeps looking at the repo its build names until it is updated once from there.
- Should the toolbox's own signing identity be pinned in the catalog, like a craft's could be? Today the first installed copy's signer is the reference for later updates.
