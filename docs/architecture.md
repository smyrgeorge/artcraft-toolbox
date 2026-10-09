# ArtCraft Toolbox architecture

Status: v1 (2026-10-09). The crates marked *planned* are designed here and registered in
`xtask/src/layers.rs`, but not written yet; `docs/roadmap.md` says when each one lands.

## 1. Goals and principles

**Goals**

- One native app (macOS, Windows, Linux) that lists every Crafting App, installs it, keeps it up
  to date, rolls it back, and launches it: one place to manage the whole suite.
- Every craft stays independent. The toolbox manages them; it is never required to run one, and
  an app installed by hand keeps working.
- Agents can do everything a person can (CLI today, control channel and MCP later).

**Principles** (the same as PhotoCraft's, where they apply)

1. **Engine-first, headless-first.** Every feature is reachable without a GUI: from tests, the CLI
   and later MCP. The GUI is one client of the engine.
2. **Everything is a command.** Each action has a stable id (`app.install`, `apps.status`) with
   JSON params, dispatched through `Session::execute`.
3. **Data, not code, describes the suite.** The apps are `crates/catalog/catalog.toml`; how they
   publish is `docs/release-contract.md`. Adding a craft is a catalog entry, not a code change.
4. **A pure core.** Parsing and decisions (versions, asset names, checksums, feeds, statuses,
   update plans) take bytes and return data. Only L3+ touches the network or the disk, so the
   core is tested with real captured responses, deterministically and offline.
5. **Verify, then act.** Nothing downloaded is opened before its SHA-256 matches the release's
   `SHA256SUMS.txt`; nothing installed replaces the working version until it is in place.
6. **Clean-room.** We study other app managers for behaviour and look only.
   Never copy their code or assets.

## 2. Crate map

```text
artcraft-toolbox/
├─ Cargo.toml                 [workspace], shared deps, lints, profiles
├─ crates/
│  ├─ release/    L0 standalone  Version (semver order), asset names, Target (os/arch), SHA256SUMS
│  ├─ catalog/    L0 standalone  App { id, name, tagline, repo, former_slugs, … } + catalog.toml
│  ├─ model/      L1             Inventory (installed versions, active one), Settings (channel, …)
│  ├─ feed/       L2             GitHub "list releases" JSON -> Release; Status per app; latest()
│  ├─ net/        L3 planned     HTTPS client: GitHub-only, redirects to GitHub's CDN only, ETag,
│  │                             size caps, resume, progress callbacks (native only)
│  ├─ install/    L3 planned     per-OS: mount DMG + copy .app; extract portable zip; place AppImage;
│  │                             shortcuts / desktop entries; uninstall; launch; running-app check
│  ├─ jobs/       L4 planned     background jobs: download -> verify -> install -> record; self-update
│  ├─ engine/     L5             Session (catalog, inventory, settings, feeds, host) + commands
│  ├─ ui-egui/    L6             ToolboxApp: app list, settings; theme tokens, widgets
│  ├─ automation/ L6 planned     JSON control channel + MCP server over the command registry
│  └─ testkit/       planned     fake feeds, temp install roots, a local HTTPS fixture server
├─ apps/
│  ├─ artcraft-toolbox/          desktop binary (eframe + wgpu + ui-egui)
│  └─ artcraft-toolbox-cli/      headless CLI over the same engine
└─ xtask/                        layers, ci, contract (live release check), version
```

**What exists today** (M0): `release`, `catalog`, `model`, `feed`, `engine`, `ui-egui`, both apps
and xtask. The engine answers "what is installed, what is available, what needs updating" from a
feed it is given; fetching feeds is M1 and installing is M2.

## 3. Layers (enforced)

```text
 L7  apps/*                         binaries: wire everything together
 L6  ui-egui · automation           frontends' libraries (UI crates allowed from here)
 L5  engine                         Session + commands
 L4  jobs                           multi-step work in the background
 L3  net · install                  the machine: network, disk, OS (I/O crates allowed from here)
 L2  feed                           pure: releases and statuses
 L1  model                          pure: toolbox state
 L0  release · catalog              pure, standalone: no workspace dependencies
```

`cargo xtask layers` fails on: an upward or sideways dependency, a workspace dependency of a
standalone crate, a UI crate (egui, eframe, winit, rfd, …) below L6, an I/O crate (ureq, reqwest,
tokio, zip, …) below L3, testkit as a normal dependency, or an unregistered crate.

## 4. Data flow

```text
catalog.toml ─────────────► Catalog ───────────────────────────────┐
GitHub /repos/<repo>/releases ─(net)─► feed::parse_releases ─► Feed ┤
inventory.json ───────────► Inventory (installed, active version) ──┼─► feed::status ─► AppStatus
settings.json ────────────► Settings (channel, keep_previous, …) ───┘     (UI rows, `apps.status`)

app.install / app.update (M2/M3):
  feed::latest(releases, channel, host) ─► Asset + SHA256SUMS.txt URL
    ─(net)─► staged file ─► SHA-256 == SHA256SUMS entry? ─► platform signature ok? (M3)
    ─(install)─► stage next to the target ─► rename into place ─► Inventory::record ─► inventory.json
```

Asset choice is `release::asset::select`: the most native architecture first (Apple silicon takes
the universal DMG; Windows on ARM takes arm64, falling back to x64 emulation for releases that
predate arm64 builds), then the preferred per-user package for the OS
(`PackageKind::preferred`): DMG on macOS, the portable zip on Windows (then MSI), AppImage on
Linux (then tar.gz), tar.gz on FreeBSD.

## 5. Install layout (proposal, decided in M2)

The toolbox's own data directory (settings, inventory, feed cache, logs), following PhotoCraft's
`app_dirs.rs`: `ARTCRAFT_TOOLBOX_CONFIG_DIR` when set; else macOS
`~/Library/Application Support/ArtCraft Toolbox`, Windows `%APPDATA%\ArtCraft Toolbox`, Linux
`$XDG_CONFIG_HOME/artcraft-toolbox` (`~/.config/artcraft-toolbox`).

| OS | Versions kept in | Active version exposed as | Notes |
|---|---|---|---|
| macOS | `~/Library/Application Support/ArtCraft Toolbox/apps/<id>/<version>/<Name>.app` | `~/Applications/<Name>.app` (moved in on activate, so Spotlight, Launchpad and the Dock see a real bundle) | DMG mounted read-only with `hdiutil attach -nobrowse -readonly`, `.app` copied with `ditto`, detached; quarantine handled per Gatekeeper rules (M3) |
| Windows | `%LOCALAPPDATA%\ArtCraft\<Name>\<version>\` | Start Menu shortcut `ArtCraft\<Name>.lnk` to the active version | Portable zip: **delete `portable.txt`** after extracting, so app data stays in `%APPDATA%\<Name>` across versions (release contract › Gotchas) |
| Linux | `~/.local/share/artcraft-toolbox/apps/<id>/<version>/<id>.AppImage` | `~/.local/share/applications/ai.storyteller.<id>.desktop` + hicolor icons | AppImages need FUSE 2 (or `APPIMAGE_EXTRACT_AND_RUN=1`); the desktop entry must match the app's `APP_ID` so Wayland docks find its icon |

Rollback is switching the active version; `Settings::keep_previous` versions stay on disk.
`Settings::install_dir` overrides the versions root.

## 6. The engine

`Session` owns the catalog, inventory, settings, the per-app feeds and the host `Target`.
`Session::execute(id, params)` finds the command, checks its `enabled` precondition, requires a JSON
object for params, runs it, and turns an escaped panic into `EngineError::Internal`. Typed
accessors (`statuses()`, `app_status()`) serve the UI; commands serve everyone else.

| Command | Params | Since |
|---|---|---|
| `catalog.list` | `{}` | M0 |
| `app.info` | `{"app":"<id>"}` | M0 |
| `app.status` | `{"app":"<id>"}` | M0 |
| `apps.status` | `{}` | M0 |
| `settings.get` | `{}` | M0 |
| `settings.set` | any subset of `channel`, `checkIntervalHours`, `autoUpdate`, `keepPrevious`, `installDir` | M0 |
| `updates.check` | `{"app"?:"<id>"}` | M1 |
| `app.install`, `app.uninstall`, `app.launch` | `{"app":"<id>","version"?:"x.y.z"}` | M2 |
| `app.update`, `apps.updateAll`, `app.rollback` | `{"app":"<id>"}` / `{}` | M3 |

Long work (downloads, installs) will run as background jobs with progress, the way PhotoCraft's
`Session` runs long commands (`crates/engine/src/jobs.rs` there).

## 7. Self-update

The toolbox is distributed exactly like a craft (M5: PhotoCraft's release pipeline, ported) and
follows the same release contract, so it can update itself through its own feed: download,
verify, stage, and swap on next start (the running binary can't replace itself on Windows).
