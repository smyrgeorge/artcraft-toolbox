# ArtCraft Toolbox architecture

Status: v1.3 (2026-10-09, M1, M2 and M4). The crates marked *planned* are designed here and registered in
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
│  ├─ net/        L3             HTTPS (ureq + rustls, OS trust store): host policy checked per
│  │                             redirect hop, token scoping, size caps, timeouts, rate-limit headers
│  ├─ store/      L3             the toolbox's files: data dir, atomic writes, settings, inventory,
│  │                             feed cache
│  ├─ install/    L3             per-OS: mount DMG + copy .app; extract portable zip; place AppImage;
│  │                             Start Menu shortcut / desktop entry; uninstall; launch; running check
│  ├─ jobs/       L4 planned     background work outside the engine (self-update); installs are
│  │                             engine jobs today
│  ├─ engine/     L5             Session (catalog, inventory, settings, feeds, host) + commands
│  ├─ ui-egui/    L6             ToolboxApp: app list, settings; theme tokens, widgets
│  ├─ automation/ L6 planned     JSON control channel + MCP server over the command registry
│  └─ testkit/       planned     fake feeds, temp install roots, a local HTTPS fixture server
├─ apps/
│  ├─ artcraft-toolbox/          desktop binary (eframe + wgpu + ui-egui)
│  └─ artcraft-toolbox-cli/      headless CLI over the same engine
└─ xtask/                        layers, ci, contract (live release check), version
```

**What exists today** (M1, M2, M4): `release`, `catalog`, `model`, `feed`, `net`, `store`,
`install`, `engine`, `ui-egui`, both apps and xtask. The toolbox checks GitHub for every app's
releases in the background, caches them, persists its settings and inventory, shows what is
available and what needs updating, and installs, uninstalls and opens any craft (one version per
app; updating an installed app is M3).

## 3. Layers (enforced)

```text
 L7  apps/*                         binaries: wire everything together
 L6  ui-egui · automation           frontends' libraries (UI crates allowed from here)
 L5  engine                         Session + commands
 L4  jobs                           multi-step work in the background
 L3  net · store · install          the machine: network, disk, OS (I/O crates allowed from here)
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
GitHub /repos/<repo>/releases?per_page=20 ─(net, If-None-Match)─►   │
   worker: feed::parse_releases ─► feeds/<app>.json ─► Feed ────────┤
inventory.json ───────────► Inventory (installed, active version) ──┼─► feed::status ─► AppStatus
settings.json ────────────► Settings (channel, keep_previous, …) ───┘     (UI rows, `apps.status`)

app.install (M2) / app.update (M3), one background job:
  feed::latest(releases, channel, pin, host) ─► Asset + its release's SHA256SUMS.txt
    ─(net)─► SHA256SUMS.txt first: no entry for the asset, no download
    ─(net, Range resume)─► downloads/<file>.part ─► exact listed size? SHA-256 == entry?
    ─► platform signature ok? (M3)
    ─(install)─► stage next to the target ─► rename into place ─► Inventory::record ─► inventory.json
```

Asset choice is `release::asset::select`: the most native architecture first (Apple silicon takes
the universal DMG; Windows on ARM takes arm64, falling back to x64 emulation for releases that
predate arm64 builds), then the preferred per-user package for the OS
(`PackageKind::preferred`): DMG on macOS, the portable zip on Windows (then MSI), AppImage on
Linux (then tar.gz), tar.gz on FreeBSD.

## 5. Data directory and install layout

The toolbox's own data directory (`store::dirs`, from PhotoCraft's `app_dirs.rs`):
`ARTCRAFT_TOOLBOX_CONFIG_DIR` when set; else macOS `~/Library/Application Support/ArtCraft Toolbox`,
Windows `%APPDATA%\ArtCraft Toolbox`, Linux `$XDG_CONFIG_HOME/artcraft-toolbox`
(`~/.config/artcraft-toolbox`).

```text
settings.json          Settings, with per-app overrides (replaced atomically; unreadable → backed up as .corrupt, defaults used)
inventory.json         what is installed where (unreadable → reported, locked, never overwritten)
state.json             what is remembered: the update versions already notified
feeds/<app>.json       the last release feed: raw GitHub body, ETag, check time
icons/<app>.png        the app's icon, and icons/<app>.json (ETag, fetch time)
logs/                  artcraft-toolbox.log, .1.log, .2.log (desktop app)
```

Installed apps (decided in M2; `install::Layout::platform`):

| OS | Installed as | Also | How |
|---|---|---|---|
| macOS | `~/Applications/<Name>.app` | | DMG attached read-only (`hdiutil attach -nobrowse -readonly -noautoopen`, private mount point), its one `.app` checked against the catalog's bundle id (`plutil`), copied with `ditto` under a staging name, renamed into place; detached whatever happens. Spotlight, Launchpad and the Dock see a real bundle |
| Windows | `%LOCALAPPDATA%\Programs\ArtCraft\<Name>\<version>\<id>.exe` | Start Menu `ArtCraft\<Name>.lnk` | Portable zip, extracted as hostile input (below); **`portable.txt` deleted**, so app data stays in `%APPDATA%\<Name>` across versions (release contract › Gotchas). The shortcut is made by PowerShell's `WScript.Shell`, paths passed in environment variables |
| Linux | `~/.local/share/artcraft-toolbox/apps/<id>/<version>/<id>.AppImage` (mode 755) | `~/.local/share/applications/<bundle id>.desktop`, `~/.local/share/icons/hicolor/128x128/apps/<bundle id>.png` | Checked to be a type 2 AppImage. The entry is marked `X-ArtCraft-Toolbox=true` (someone else's entry under that name is left alone) and named after the app's `APP_ID`, so Wayland docks find its icon. AppImages need FUSE 2 (or `APPIMAGE_EXTRACT_AND_RUN=1`) |

Windows takes the portable zip, not the MSI: versions side by side, no installer UI, no
elevation (roadmap › Open questions keeps the MSI question).

The rules, on every OS:

- **Staged, then renamed.** Everything goes in under a hidden staging name beside its target and
  is renamed into place, so an interrupted install leaves nothing half-done.
- **Nothing is replaced.** If the target exists (an app installed by hand, or by an earlier
  toolbox), the install fails with "already exists"; adopting such apps is M3.
- **Uninstall removes exactly what install made**, after checking that the recorded path still has
  the shape install gave it, and refuses a running app (`pgrep` on macOS and Linux; on Windows the
  locked `.exe` makes the removal fail). Documents and the app's own settings stay.
- **Archives are hostile input.** Zip entries with absolute, drive or `..` paths, symbolic links
  and duplicate names are refused; entry count (10,000) and unpacked size (4 GiB) are capped, and
  an entry can't write more than it declares.
- **Downloads** are staged in `<data>/downloads/<file>.part`, resumed with an HTTP range after a
  cancel or a crash, and deleted once installed.

`Settings::install_dir` replaces the apps folder (the Start Menu shortcut and desktop entry stay in
their standard places). Opening an app (`app.launch`) is `open` on macOS and a detached process
elsewhere. Keeping previous versions and rolling back (`Settings::keep_previous`) are M3: on
macOS the previous bundles will be kept outside `~/Applications`.

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
| `settings.set` | any subset of `channel`, `checkIntervalHours`, `autoUpdate`, `keepPrevious`, `installDir`, `notifications`, `closeToTray` | M0, M4 |
| `app.settings.get` | `{"app":"<id>"}` | M4 |
| `app.settings.set` | `{"app":"<id>","channel"?:…\|null,"autoUpdate"?:bool\|null,"pinned"?:"x.y.z"\|null}` | M4 |
| `app.releases` | `{"app":"<id>","limit"?:1..100}` | M4 |
| `icons.refresh` (background) | `{"force"?:bool}` | M4 |
| `updates.check` (background) | `{"app"?:"<id>","force"?:bool}` | M1 |
| `app.install` (background) | `{"app":"<id>","version"?:"x.y.z"}` (default: the newest the channel and pin offer) | M2 |
| `app.uninstall`, `app.launch` | `{"app":"<id>"}` | M2 |
| `app.update`, `apps.updateAll`, `app.rollback` | `{"app":"<id>"}` / `{}` | M3 |

Long work runs as background jobs (`engine/src/jobs.rs`, PhotoCraft's pattern): a command with
a `start` hook runs on worker threads when called through `Session::start` (the UI), and to
completion through `Session::execute` (the CLI, tests). Workers never touch the session; they
send messages that `Session::poll_jobs` applies on the session's thread, so rows update as each
app's result arrives. `updates.check` runs up to 4 fetches at once, parses and caches each feed
on the worker, stops when GitHub's rate limit is reached (and keeps checking disabled until its
reset), and skips apps checked in the last minute. `icons.refresh` uses the same worker pool
(`jobs::spawn_pool`): it fetches missing or week-old icons, decodes them to RGBA on the worker
(`icons::decode_png`, size-capped) and caches the PNG. `app.install` is one job per app, on its
own worker: it reports a phase (Verifying the release, Downloading with a fraction, Verifying,
Installing) that the row and the CLI show, and checks its cancel flag between chunks; a cancelled
download keeps its `.part` file, so installing again resumes it. The session records the
installation when the job's result arrives.

## 7. Self-update

The toolbox is distributed exactly like a craft (M5: PhotoCraft's release pipeline, ported) and
follows the same release contract, so it can update itself through its own feed: download,
verify, stage, and swap on next start (the running binary can't replace itself on Windows).

## 8. The HTTP client (decided in M1)

**ureq 3 with rustls and the platform verifier.** Blocking (the job workers are plain threads;
no async runtime to carry), small, pure-Rust TLS, and certificates checked against the OS trust
store, so a corporate TLS-inspecting proxy with its own CA works as it does in the user's
browser; ureq also honours the usual `HTTPS_PROXY` variables. reqwest would bring tokio for no
benefit here; native-tls would tie TLS behaviour to each OS's stack.

ureq's own redirect following is off: `net::Client` follows redirects itself so every hop is
checked against the policy's host list before it is contacted, and the `Authorization` header
is sent only to the policy's token hosts (never along a redirect to another host).

## 9. The desktop shell: tray, notifications, a hidden window (M4)

The platform features live in the desktop app (`apps/artcraft-toolbox`), as PhotoCraft keeps its
macOS menu bar there; `ui-egui` only sees them through `Services` (a notification callback, and
whether a tray icon exists), so tests and the snapshot example run without them.

- **Ticking while hidden.** eframe 0.36 runs `App::logic` before every frame and, while the
  window is hidden, whenever a repaint is requested: no UI pass, but viewport commands (show,
  focus, close) are still processed. `ToolboxApp::tick` (poll jobs, start due work, close to tray)
  runs there and asks for a repaint every minute (`IDLE_TICK`), so checks happen with the window
  hidden.
- **Tray.** `tray-icon` 0.26 (the muda authors' crate, on the muda 0.21 PhotoCraft pins); on Linux
  its `ksni` backend (StatusNotifierItem over D-Bus, `zbus`), so no GTK. Its click handlers only
  queue an action and request a repaint; `logic` applies it. Creation can fail (no StatusNotifier
  host): the toolbox then runs without one and closing the window quits.
- **Close to tray.** A window close request is cancelled and the window hidden, while there is a
  tray, the `closeToTray` setting is on and the user didn't choose Quit.
- **Notifications.** `notify-rust`, on a short-lived thread. `Session::take_new_updates` returns
  each installed app's new version once, remembered in `state.json` across restarts.
