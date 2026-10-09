# Roadmap

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Updated 2026-10-09.

| M | Status | Goal |
|---|---|---|
| M0 Skeleton | ✅ | PhotoCraft's workspace, conventions and CI; the pure core; engine, CLI and UI shell |
| M1 Live feeds | ⬜ | Fetch, cache and show every app's releases; persist settings and inventory |
| M2 Install and launch | ⬜ | Install, uninstall and open any craft on macOS, Windows and Linux |
| M3 Updates, rollback, trust | ⬜ | Update one or all, keep and switch versions, verify signatures, adopt existing installs |
| M4 Toolbox UX parity | ⬜ | Menu-bar/tray app, background checks, notifications, release notes, per-app settings |
| M5 Distribution and self-update | ⬜ | Signed installers via PhotoCraft's release pipeline; the toolbox updates itself |
| M6 Automation | ⬜ | Control channel and MCP over the command registry |
| M7 Catalog from the network | ⬜ | A signed remote catalog; non-conforming apps (ArtCraft itself) |
| M8 Polish | ⬜ | Localization (PhotoCraft's `tl!` pattern), accessibility, light theme |

## Current focus: M1 Live feeds

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

### M1 Live feeds

- [ ] Data directory (`app_dirs.rs`, ported from PhotoCraft: `ARTCRAFT_TOOLBOX_CONFIG_DIR`, platform default; portable mode not needed)
- [ ] Load and save `settings.json` and `inventory.json` atomically (temp file + rename); a corrupt inventory is reported, never overwritten
- [ ] File logging with rotation (port PhotoCraft's `logging.rs`)
- [ ] `net` crate (L3): HTTPS GET with a GitHub-only host allowlist, redirect only to GitHub's asset CDN, response size caps, timeouts, `ArtCraft-Toolbox/<version>` User-Agent; pick the client (ureq + rustls is the likely fit: blocking, small, pure Rust) and record why
- [ ] Feed cache in the data dir with ETag / `If-None-Match` (304s don't count against the rate limit); optional `GITHUB_TOKEN`
- [ ] `updates.check` command: all apps or one, as a background job; rate-limit (403/429) back-off with a clear message
- [ ] UI: "Check for updates" button, last-checked time, per-row spinner, errors in the status bar
- [ ] Check on start when the last check is older than `checkIntervalHours`
- [ ] CLI: `artcraft-toolbox-cli check` (network) next to `status --feed` (offline)

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

### M4 Toolbox UX parity

- [ ] Menu-bar (macOS) / tray (Windows, Linux) presence, closing to tray
- [ ] Background checks and OS notifications for new versions
- [ ] Release notes view (Markdown) per version
- [ ] Per-app overrides: channel, auto-update, pinned version
- [ ] App icons from each craft's repository (cached), instead of monogram tiles

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

- [ ] Signed remote catalog (new crafts appear without a toolbox release); the built-in catalog stays the fallback
- [ ] Per-app asset patterns for apps outside the contract (ArtCraft's Tauri releases)

## Open questions

- Should Windows default to the MSI (per-user) instead of the portable zip? The zip gives side-by-side versions and no installer UI; the MSI gives Add/Remove Programs and file associations.
- Do crafts' file associations (`.pcraft`, PSD) need registering when the toolbox installs from a portable zip?
- Publishing: does the toolbox live in `storytold/` with the other crafts? Its name and app id (`ai.storyteller.toolbox`) should be confirmed with the ArtCraft team.
