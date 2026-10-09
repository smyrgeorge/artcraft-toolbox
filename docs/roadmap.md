# Roadmap

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Updated 2026-10-09.

| M | Status | Goal |
|---|---|---|
| M0 Skeleton | ✅ | PhotoCraft's workspace, conventions and CI; the pure core; engine, CLI and UI shell |
| M1 Live feeds | ✅ | Fetch, cache and show every app's releases; persist settings and inventory |
| M2 Install and launch | ⬜ | Install, uninstall and open any craft on macOS, Windows and Linux |
| M3 Updates, rollback, trust | ⬜ | Update one or all, keep and switch versions, verify signatures, adopt existing installs |
| M4 Toolbox UX parity | ✅ | Menu-bar/tray app, background checks, notifications, release notes, per-app settings |
| M5 Distribution and self-update | ⬜ | Signed installers via PhotoCraft's release pipeline; the toolbox updates itself |
| M6 Automation | ⬜ | Control channel and MCP over the command registry |
| M7 Catalog from the network | ⬜ | A signed remote catalog; non-conforming apps (ArtCraft itself) |
| M8 Polish | ⬜ | Localization (PhotoCraft's `tl!` pattern), accessibility, light theme |

## Current focus: M2 Install and launch

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

### M2 Install and launch

- [ ] `install` crate (L3) with a per-OS backend behind one trait; tests against temp dirs
- [ ] Download to a staging file with progress and resume; verify SHA-256 against `SHA256SUMS.txt` before anything else
- [ ] macOS: mount DMG read-only, copy `<Name>.app`, detach; expose in `~/Applications`
- [ ] Windows: extract the portable zip safely (zip-slip, symlinks, size caps), delete `portable.txt`, Start Menu shortcut
- [ ] Linux: place the AppImage, `chmod +x`, desktop entry and icons under `~/.local/share`
- [ ] `app.install`, `app.uninstall`, `app.launch` commands; Install / Open buttons go live
- [ ] Decide and document the install layout (docs/architecture.md § 5 is the proposal)

### M3 Updates, rollback, trust

- [ ] `app.update`, `apps.updateAll` (with "Update all" in the UI), never replacing a running app
- [ ] Keep `keepPrevious` versions; `app.rollback`; versions list per app
- [ ] macOS: `codesign --verify` + Gatekeeper assessment before activating; Windows: Authenticode status reported
- [ ] Detect apps installed by hand and offer to adopt them; re-read installed versions on refresh (release contract › Gotchas 4)
- [ ] Coordinate with crafts that check for updates themselves

### M4 Toolbox UX parity ✅ (2026-10-09, ahead of M2/M3)

- [x] Menu-bar (macOS, template glyph) / tray (Windows, Linux via StatusNotifier, no GTK) icon with Open, Check for Updates, Quit; closing the window keeps the toolbox running (setting `closeToTray`); without a tray host, closing quits
- [x] Periodic background checks while the app runs, also hidden (`eframe::App::logic` ticks every minute; automatic checks back off 15 min after any attempt), and OS notifications for new versions of installed apps, once per version (`state.json`), setting `notifications`
- [x] App details page (click a row): links, status, versions with their release notes rendered from Markdown (bare URLs linked, only web links open)
- [x] Per-app overrides: channel, auto-update, pinned version (`app.settings.get` / `app.settings.set`; the details page; pins cap what is offered, never downgrade)
- [x] App icons from each craft's repository (`raw.githubusercontent.com`, not the API quota), cached with ETags, refreshed weekly (`icons.refresh`); monogram tiles until they arrive
- [x] `app.releases` for agents: versions, notes, installable here, offered by the channel and pin

Still open from M4: macOS keeps its Dock icon while the window is hidden (an accessory activation
policy needs AppKit calls; PhotoCraft's `mac_window.rs` shows the safe way), and a notification
click doesn't open the toolbox yet. The Install / Update / Open buttons stay disabled until M2.

### M5 Distribution and self-update

- [ ] Port PhotoCraft's `packaging/` and `release.yml` (macOS universal DMG signed and notarized, Windows MSI + portable, Linux AppImage/deb/rpm)
- [ ] App icon (replace the placeholder), Windows resources (`build.rs`), Linux desktop file and AppStream metadata
- [ ] The toolbox follows its own release contract and updates itself (swap on next start)
- [ ] `packaging-lint.yml` as in PhotoCraft

### M6 Automation

- [ ] JSON control channel (port PhotoCraft's control-server pattern: localhost, bearer token)
- [ ] MCP server exposing the command registry
- [ ] Every UI state readable and settable (`ui.get`, `ui.set`)

### M7 Catalog from the network

- [ ] One aggregated, signed release feed for the whole suite (a static file, not the rate-limited API): one request per check instead of 12, and no 60/hour ceiling for anonymous users
- [ ] Signed remote catalog (new crafts appear without a toolbox release); the built-in catalog stays the fallback
- [ ] Per-app asset patterns for apps outside the contract (ArtCraft's Tauri releases)

## Open questions

- Should Windows default to the MSI (per-user) instead of the portable zip? The zip gives side-by-side versions and no installer UI; the MSI gives Add/Remove Programs and file associations.
- Do crafts' file associations (`.pcraft`, PSD) need registering when the toolbox installs from a portable zip?
- Publishing: does the toolbox live in `storytold/` with the other crafts? Its name and app id (`ai.storyteller.toolbox`) should be confirmed with the ArtCraft team.
