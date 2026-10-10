# ArtCraft Toolbox architecture

Status: v1.7 (2026-10-10, M1–M8). The crates marked *planned* are designed here and registered in
`xtask/src/layers.rs`, but not written yet; `docs/roadmap.md` says when each one lands.

## 1. Goals and principles

**Goals**

- One native app (macOS, Windows, Linux) that lists every Crafting App, installs it, keeps it up
  to date, rolls it back, and launches it: one place to manage the whole suite.
- Every craft stays independent. The toolbox manages them; it is never required to run one, and
  an app installed by hand keeps working.
- Agents can do everything a person can: the CLI, the control channel and MCP drive the same
  commands (§ 11).

**Principles** (the same as PhotoCraft's, where they apply)

1. **Engine-first, headless-first.** Every feature is reachable without a GUI: from tests, the CLI,
   the control channel and MCP. The GUI is one client of the engine.
2. **Everything is a command.** Each action has a stable id (`app.install`, `apps.status`) with
   JSON params, dispatched through `Session::execute`.
3. **Data, not code, describes the suite.** The apps are `crates/catalog/catalog.toml`; how they
   publish is `docs/release-contract.md`. Adding a craft is a catalog entry, not a code change,
   and since M7 not even a release: the publisher's signed catalog reaches every toolbox (§ 12).
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
│  ├─ ui-egui/    L6             ToolboxApp: app list, details, settings; theme tokens (dark,
│  │                             light), Inter, Lucide icons, widgets; i18n (catalogs, `tl!`),
│  │                             wording, CJK fonts
│  ├─ automation/ L6             the JSON-lines control protocol (tokens, limits, the headless
│  │                             server, the bridge client) and the MCP server (rmcp) over the
│  │                             command registry
│  └─ testkit/       planned     fake feeds, temp install roots, a local HTTPS fixture server
├─ apps/
│  ├─ artcraft-toolbox/          desktop binary (eframe + wgpu + ui-egui); `--control <port>`
│  │                             serves the window to agents
│  └─ artcraft-toolbox-cli/      headless CLI over the same engine; `serve` and `mcp`
└─ xtask/                        layers, ci, contract (live release check), version
```

**What exists today** (M1–M6, M8): `release`, `catalog`, `model`, `feed`, `net`, `store`,
`install`, `engine`, `ui-egui`, `automation`, both apps and xtask. The toolbox checks GitHub for every app's
releases in the background, caches them, persists its settings and inventory, shows what is
available and what needs updating, and installs, updates (by hand or automatically), rolls back,
adopts, uninstalls and opens any craft, checking platform signatures on the way. Its window is in
15 languages, in a dark or light theme, at a chosen text size. It is packaged like a craft
(`docs/releasing.md`) and updates itself through its own release feed (§ 7). Agents drive it
through the control channel and MCP (§ 11). The `jobs` crate stayed planned: installs and the
self-update are engine jobs.

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
catalog.toml ─────────────► Catalog ◄── remote/catalog.json (signed, revision ≥) ──┐
raw…/artcraft-catalog.json ─(net, If-None-Match)─► signing::verify ─► Catalog::accepts │
raw…/artcraft-feed.json ───(net, If-None-Match)─► signing::verify ─► per app ─────────┤
GitHub /repos/<repo>/releases?per_page=20 ─(net, If-None-Match)─► (what the feed misses) │
   worker: feed::parse_releases_for ─► feeds/<app>.json ─► Feed ────┤
inventory.json ───────────► Inventory (installed, active version) ──┼─► feed::status ─► AppStatus
settings.json ────────────► Settings (channel, keep_previous, …) ───┘     (UI rows, `apps.status`)

app.install / app.update, one background job:
  feed::latest(releases, channel, pin, host) ─► Asset + its release's SHA256SUMS.txt
    ─(net)─► SHA256SUMS.txt first: no entry for the asset, no download
    ─(net, Range resume)─► downloads/<file>.part ─► exact listed size? SHA-256 == entry?
    ─(install)─► placed beside the active version ─► platform signature intact? same developer
    as the version it replaces?
    ─(install)─► activate (macOS: swap bundles) ─► prune beyond keepPrevious ─► Inventory::record
    ─► inventory.json
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
state.json             what is remembered: the update versions already notified, the toolbox's own staged update
feeds/<app>.json       the last release feed: raw GitHub body, ETag, check time (the toolbox's own under artcraft-toolbox.json)
icons/<app>.png        the app's icon, and icons/<app>.json (ETag, fetch time)
downloads/             partial and verified downloads, until they are installed
self-update/           a newer toolbox placed for the next start, and previous/ with the replaced one (§ 7)
logs/                  artcraft-toolbox.log, .1.log, .2.log (desktop app)
```

Installed apps (`install::Layout::platform`):

| OS | Installed as | Also | How |
|---|---|---|---|
| macOS | `~/Applications/<Name>.app` (the version in use) | other versions in `<data>/versions.noindex/<id>/<version>/<Name>.app` | DMG attached read-only (`hdiutil attach -nobrowse -readonly -noautoopen`, private mount point), its one `.app` checked against the catalog's bundle id (`plutil`), copied with `ditto` into the kept folder; detached whatever happens. Activating moves the bundle in use out to its version folder and the new one in (back again if that fails). Spotlight, Launchpad and the Dock see one real bundle; `.noindex` keeps Spotlight away from the others |
| Windows | `%LOCALAPPDATA%\Programs\ArtCraft\<Name>\<version>\<id>.exe`, versions side by side | Start Menu `ArtCraft\<Name>.lnk` to the version in use | Portable zip, extracted as hostile input (below); **`portable.txt` deleted**, so app data stays in `%APPDATA%\<Name>` across versions (release contract › Gotchas). The shortcut is made by PowerShell's `WScript.Shell`, paths passed in environment variables |
| Linux | `~/.local/share/artcraft-toolbox/apps/<id>/<version>/<id>.AppImage` (mode 755), versions side by side | `~/.local/share/applications/<bundle id>.desktop` running the version in use, `~/.local/share/icons/hicolor/128x128/apps/<bundle id>.png` | Checked to be a type 2 AppImage. The entry is marked `X-ArtCraft-Toolbox=true` (someone else's entry under that name is left alone) and named after the app's `APP_ID`, so Wayland docks find its icon. AppImages need FUSE 2 (or `APPIMAGE_EXTRACT_AND_RUN=1`) |

Windows takes the portable zip, not the MSI: versions side by side, no installer UI, no
elevation (roadmap › Open questions keeps the MSI question).

The rules, on every OS:

- **Staged, then renamed.** Everything goes in under a hidden staging name beside its target and
  is renamed into place, so an interrupted install leaves nothing half-done.
- **Placed, checked, then activated.** A new version goes in beside the one in use; only after
  its checksum and platform signature pass is it made the one the user opens. A failure removes
  it again and leaves the version in use as it was.
- **Platform signatures** (`install::trust`): on macOS `codesign --verify --deep` (a changed
  file or executable fails) and Gatekeeper (`spctl --assess`, "notarized"); on Windows
  Authenticode (`Get-AuthenticodeSignature`); AppImages carry none. A broken signature is refused;
  a missing one is shown. The developer that signed the version in use (macOS team id, else the
  certificate subject) is recorded, and **an update signed by anyone else is refused**.
- **Nothing is replaced.** If the target exists (an app installed by hand, or by an earlier
  toolbox), installing fails with "already exists"; `apps.rescan` finds such copies and
  `app.adopt` takes them over (signature checked first).
- **Removal removes exactly what install made**, after checking that the recorded path still has
  the shape install gave it, and refuses a running app (`pgrep` on macOS and Linux, the processes'
  image paths on Windows). Uninstall removes the kept versions too. Documents and the app's own
  settings stay.
- **Archives are hostile input.** Zip entries with absolute, drive or `..` paths, symbolic links
  and duplicate names are refused; entry count (10,000) and unpacked size (4 GiB) are capped, and
  an entry can't write more than it declares.
- **Downloads** are staged in `<data>/downloads/<file>.part`, resumed with an HTTP range after a
  cancel or a crash, and deleted once installed.

`Settings::install_dir` replaces the apps folder (the Start Menu shortcut and desktop entry stay in
their standard places). Opening an app (`app.launch`) is `open --env` on macOS and a detached
process elsewhere; the app finds `ARTCRAFT_TOOLBOX_MANAGED=1` and none of the toolbox's own
`ARTCRAFT_TOOLBOX_*` variables (release contract › Managed apps).

**Updates, rollback, versions.** `app.update` installs the newest version the app's channel and
pin offer (or any release asked for) beside the one in use, and keeps the replaced one: after an
update, `keepPrevious` versions stay (the one just replaced first, then the most recently
installed), older ones are removed. `app.rollback` switches to a kept version without a download
(on macOS the bundles swap places in under a second). An update to a version that is kept is a
switch, not a download (`apps.updateAll` does that by itself). On macOS an update refuses while
the app runs (its bundle would move); on Windows and Linux the new version goes in beside the
running one, and only pruning waits. Apps set to `autoUpdate` are updated after each check
(`apps.updateAll {"onlyAutomatic":true}`) and announced when done, not when found.

**The disk is the truth** (`Session::rescan`, at start, after uninstall and adopt, and on
`apps.rescan`): copies installed outside the toolbox are found (for apps it hasn't installed),
records whose files are gone are forgotten (when the folder around them is still there: an
unplugged drive is not a deletion), and on macOS an app that updated itself is recorded at the
version its bundle now says (release contract › Gotchas 4).

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
| `settings.set` | any subset of `channel`, `checkIntervalHours`, `autoUpdate`, `keepPrevious`, `installDir`, `notifications`, `closeToTray`, `language`, `theme`, `textSize` | M0, M4, M8 |
| `app.settings.get` | `{"app":"<id>"}` | M4 |
| `app.settings.set` | `{"app":"<id>","channel"?:…\|null,"autoUpdate"?:bool\|null,"pinned"?:"x.y.z"\|null}` | M4 |
| `app.releases` | `{"app":"<id>","limit"?:1..100}` | M4 |
| `icons.refresh` (background) | `{"force"?:bool}` | M4 |
| `updates.check` (background) | `{"app"?:"<id>","force"?:bool}` | M1 |
| `app.install` (background) | `{"app":"<id>","version"?:"x.y.z"}` (default: the newest the channel and pin offer) | M2 |
| `app.uninstall`, `app.launch` | `{"app":"<id>"}` | M2 |
| `app.update` (background) | `{"app":"<id>","version"?:"x.y.z"}` | M3 |
| `app.rollback` | `{"app":"<id>","version"?:"x.y.z"}` (default: the newest kept version older than the one in use) | M3 |
| `app.versions` | `{"app":"<id>"}` | M3 |
| `app.adopt` | `{"app":"<id>"}` | M3 |
| `apps.updateAll` | `{"onlyAutomatic"?:bool}` | M3 |
| `apps.rescan` | `{}` | M3 |
| `toolbox.status` | `{}` | M5 |
| `toolbox.update` (background) | `{"version"?:"x.y.z"}` | M5 |
| `toolbox.apply` | `{}` | M5 |
| `catalog.status` | `{}` | M7 |

Long work runs as background jobs (`engine/src/jobs.rs`, PhotoCraft's pattern): a command with
a `start` hook runs on worker threads when called through `Session::start` (the UI), and to
completion through `Session::execute` (the CLI, tests). Workers never touch the session; they
send messages that `Session::poll_jobs` applies on the session's thread, so rows update as each
app's result arrives. `updates.check` runs up to 4 fetches at once, parses and caches each feed
on the worker, stops when GitHub's rate limit is reached (and keeps checking disabled until its
reset), and skips apps checked in the last minute. `icons.refresh` uses the same worker pool
(`jobs::spawn_pool`): it fetches missing or week-old icons, decodes them to RGBA on the worker
(`icons::decode_png`, size-capped) and caches the PNG. `app.install` and `app.update` are one job
per app, on its own worker: it reports a phase (Verifying the release, Downloading with a
fraction, Verifying, Installing, Checking the signature) that the row and the CLI show, and
checks its cancel flag between chunks; a cancelled download keeps its `.part` file, so the next
attempt resumes it. The worker also activates the version and removes the ones beyond
`keepPrevious`; the session records it all when the job's result arrives.

## 7. Self-update (M5)

The toolbox is distributed exactly like a craft (PhotoCraft's release pipeline, ported:
`docs/releasing.md`) and follows the same release contract, so it updates itself through its
own feed with the same code that updates a craft.

- **Its feed is one more entry of every check.** `catalog.toml` has a `[toolbox]` entry (id
  `artcraft-toolbox`, repo, bundle id `ai.storyteller.toolbox`); `updates.check` fetches it with
  the apps' (13 requests per full check), caches it as `feeds/artcraft-toolbox.json`, and
  `Session::self_status` compares the running version (`build_info::VERSION`) with it on the
  global channel. The UI offers the update at the top of the app list and on the About card;
  a notification announces it once, like an app's.
- **Only a packaged copy updates itself.** At start the session locates its own installation
  (`install::selfupdate::locate`): the `.app` bundle the executable sits in, the AppImage
  (`APPIMAGE` from its runtime), or `artcraft-toolbox.exe` beside the running exe. A `cargo run`
  build, a deb/rpm/MSI install (`/usr/bin`, Program Files: not writable without elevation, and
  the toolbox never elevates) or a CLI unzipped on its own has no such installation: the
  status says so and the update is left to the way the copy was installed.
- **`toolbox.update`** (a background job, `toolbox_cmds`) is the install pipeline pointed at the
  toolbox: `SHA256SUMS.txt` first, download to `downloads/`, SHA-256, the platform installer
  places the new version in `<data>/self-update/` (the DMG attached and its one `.app` checked
  against the bundle id; the portable zip extracted as hostile input; the AppImage checked),
  its platform signature is checked and **must match the running copy's developer** (the
  running copy's own signature is read first; a broken one refuses the update), and the result
  is recorded as *staged* in `state.json`. `autoUpdate` downloads it by itself after a check
  and announces it when ready. Before any of that, the installation's folder is probed for
  write access.
- **Swap on the next start.** A program can't replace itself safely while it runs, so the swap
  (`install::selfupdate::swap`) happens when the desktop app starts, before it opens its
  window (`Session::apply_staged_update`): the installation is moved to
  `self-update/previous/` and the staged version put in its place, the new version is started
  (`Applied::relaunch`: `open -n` of the bundle on macOS, a detached process elsewhere) and the
  old one exits. On Windows a running exe can be renamed but not deleted, so there the old
  `artcraft-toolbox.exe` becomes `artcraft-toolbox.exe.previous` beside the new one (the CLI exe
  too) and the leftovers are removed at the following start. `Restart to update` in the UI does
  the same at once. `toolbox.apply` does it for the CLI and agents while the toolbox isn't
  running. A stale record (a version no newer than the one running, files gone) is forgotten.
  Nothing elevates: an installation the user can't write to refuses before downloading.
- **The previous version is kept** for a manual recovery; there is no `toolbox.rollback` yet
  (`docs/roadmap.md`).

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
- **Popover.** On macOS and Windows the window is a popover (`popover.rs`): undecorated, above
  other windows, out of the taskbar, started hidden and shown under (or above) the tray icon at
  the position `TrayIconEvent::Click` reports (or `TrayIcon::rect` at start), converted from
  physical pixels to points; placement is a pure, tested function. A left click toggles it, the
  menu moves to the right button. It hides on losing the focus; a tray click within 0.5 s of that
  is the same click and leaves it hidden. On macOS the window is transparent so the UI can round
  its corners (`Services::transparent`), and the app is an accessory (no Dock icon, no app menu,
  so Cmd+Q is handled by the app) through eframe's `event_loop_builder` hook and winit's safe
  `with_activation_policy`. If the tray icon can't be made, the window turns back into a normal
  one. Linux keeps a normal window (Wayland lets no app place its window).
- **Notifications.** `notify-rust`, on a short-lived thread. `Session::take_new_updates` returns
  each installed app's new version once, remembered in `state.json` across restarts.

## 10. Languages, themes and accessibility (M8)

Translation happens only in `ui-egui`, at display. The engine, its errors and the CLI stay
English (agents and logs read them), and the settings store codes, not words (`language: "de"`,
`theme: "light"`). The UI translates three kinds of text:

- its own literals, through `tl!` and the TSV catalogs in `ui-egui/src/i18n/` (PhotoCraft's
  implementation, ported: embedded with `include_str!`, parsed once, looked up per frame);
- what it builds from engine data, in `wording` (status lines, "5 min ago", the signature line,
  rate-limit notices), which matches on the engine's typed values rather than its English text;
- the engine's fixed messages (disabled reasons, job phases), translated by exact match from
  `wording::ENGINE_STRINGS`; a test checks each still appears in the engine's source, so a reworded
  engine message can't silently fall back to English.

Each frame, `sync_appearance` sets the drawing language (thread-local, `i18n::set_current`), the
theme preference and the zoom factor from the settings, so a `settings.set` from the UI, the CLI
or an agent shows on the next frame. A language change also resets the CJK font loader
(`cjk_fonts`), which loads the system's Chinese, Japanese or Korean fonts on demand, the UI
language's script first. The desktop app rebuilds its tray menu labels on a language change
(`Tray::relabel`).

Themes are two `Tokens` sets and their egui visuals, both installed at start; egui picks one from
the preference (`system` follows the OS). Accessibility comes from egui's AccessKit tree: widgets
get names (`WidgetInfo`), and the UI tests query that same tree.

## 11. Automation (M6)

Three doors into the same engine, all dispatching commands by id (`docs/control-protocol.md`,
`docs/mcp.md`):

- **The control channel of the desktop app.** `artcraft-toolbox --control <port>` starts a
  loopback TCP server (`apps/artcraft-toolbox/src/control_server.rs`, PhotoCraft's pattern): one
  thread accepts, one per connection reads bounded JSON lines, the first frame must carry the
  bearer token, and every later request becomes a `ControlRequest` on a channel the window owns.
  `ToolboxApp::tick` drains that channel between frames (`drain_control`, so requests run on the
  UI thread with the window's state in hand, even while the window is hidden), and the handlers in
  `ui-egui/src/control.rs` answer: `engine.execute` through `Session::start`, so a background
  command's request waits in `control_waiters` and is answered by `poll` when its job's event
  arrives; `ui.get`/`ui.set` read and write `UiState` and the banner, validating every field
  before applying any; `app.quit` closes the window like its button. `ControlRequest` lives in
  `ui-egui` because a crate may not depend sideways on `automation`; the transport primitives it
  shares with the headless server (tokens, bounded reads, the connection limiter, the reply
  budget) live in `automation::security` and `automation::budgets`, which only the app crate
  (L7) pulls in.
- **The headless server.** `artcraft-toolbox-cli serve` wraps a `Session` in
  `automation::Headless` and answers the same envelope on stdio or loopback TCP
  (`automation::rpc`). `Headless::handle` is the method table (`engine.execute`,
  `engine.commands`, `jobs.list`, `jobs.cancel`, `batch`, `methods`); a background command runs to
  completion with `Session::wait_job` unless `wait: false`, and every request first applies the
  jobs that have finished. A poisoned session lock is recovered, so one panicking command (already
  caught by `Session::execute`) never wedges the server.
- **The MCP server.** `automation::ToolboxMcp` (rmcp's `#[tool_router]`) exposes nine tools
  and two resources over a `Backend`: `Headless` (the session on a blocking thread) or `Bridge`
  (`automation::BridgeClient`, a tokio client of the control protocol: one authenticated
  connection, loopback only, 60-second replies, never a resend). The tool schemas are rewritten
  for strict validators (`$ref`s inlined, `additionalProperties: false`), unknown arguments are
  refused before execution, a malformed line gets `-32700` and the next one is served
  (`server/transport.rs`), and every result is checked against the reply budget as encoded JSON.

What an agent can't do is what the user can't do without elevation: nothing in these servers
bypasses the engine's verification, and nothing elevates. The limits (1 MiB requests, 8 MiB
replies, 16 connections, 256 batch steps) are constants in `automation::security` and
`automation::budgets`, tested in both directions.

## 12. The catalog from the network (M7)

Three things come from the publisher instead of from a toolbox release, all as signed documents
(`docs/release-contract.md` › The aggregated feed and the remote catalog):

- **The remote catalog.** The same `catalog.toml`, with a `revision`. Every `updates.check`
  starts with one conditional GET of it (`engine::remote`, on the check job's coordinator
  thread, before the per-app workers). The envelope is verified against the key pinned in the
  built-in catalog (`release::signing`, Ed25519, the kind in the signed bytes), parsed with the
  same validation as the built-in one, and accepted only if `Catalog::accepts` it: not older,
  and keeping a `[remote]` section with the same key. The session swaps its catalog on its own
  thread (`Session::set_catalog`); rows, feeds, icons and settings follow by id. The verified
  envelope is cached (`remote/catalog.json`) and applied again at the next start, offline, so a
  new craft stays visible. Anything else keeps the built-in catalog and is logged.
- **The aggregated feed.** One document with every app's GitHub "list releases" response,
  trimmed to the fields the parser reads. One conditional GET from `raw.githubusercontent.com`
  (not the API: no 60-per-hour limit, and a 304 costs nothing) replaces one API request per app;
  each list is parsed, cached per app and applied exactly as a direct response, so everything
  downstream is unchanged. Apps the feed doesn't cover, or the whole set when the feed can't be
  fetched or verified, go to the API as before. `CheckSummary` says what happened
  (`catalog`, `feed`, `aggregated`).
- **Apps outside the contract.** `[app.assets]` patterns name an app's builds per target;
  the feed generator hashes each matched build and writes `sha256` on it, and `app.install`
  takes that digest in place of `SHA256SUMS.txt` (`install_cmds::Verify`). Without either,
  nothing is installed. ArtCraft itself is listed this way.

Publishing is `cargo xtask feed` (`xtask/src/feed.rs`: fetch every app's releases with the
toolbox's own parsers, trim, hash, sign, verify its own output) run hourly by
`.github/workflows/feed.yml` onto the `feed` branch; `cargo xtask keygen` makes the key pair and
`docs/releasing.md` › The feed says where the secret goes. The setting `remoteCatalog` (default
on) turns the whole thing off; `catalog.status` reports the source, revision and times.

What this does not change: downloads still come only from GitHub release URLs, every build is
still verified before it is opened, the platform signature is still checked, and a publisher
key can't be rotated by a remote document (a new key needs a toolbox release).
