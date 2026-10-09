# AGENTS.md: guide for AI agents and contributors

ArtCraft Toolbox installs, updates, rolls back and launches the **Crafting Apps** (PhotoCraft, VectorCraft, FilmCraft, LightCraft, PdfCraft, EffectCraft, DesignCraft, CADCraft, DeckCraft, GridCraft, SoundCraft, WordCraft) from one window. It is written in **Rust only** (no JavaScript or TypeScript) on the same stack as PhotoCraft: native egui/eframe on wgpu. **No Tauri, Electron or webview shells**, and no web build (a browser can't install native apps). The product name is always written **ArtCraft Toolbox** in user-facing text; craft names are `{Function}Craft` in PascalCase (PhotoCraft, never Photocraft). Machine names stay lowercase: crates `artcraft-toolbox-*`, binaries `artcraft-toolbox` and `artcraft-toolbox-cli`, env vars `ARTCRAFT_TOOLBOX_*`, app id `ai.storyteller.toolbox`.

**PhotoCraft is the reference implementation** of every convention here (workspace, lints, layering, never-crash, xtask, CI, release pipeline). A sibling checkout is expected at `../photocraft`. When this file doesn't answer a question, look at how PhotoCraft does it and do the same, adapted; repos don't share code, so port the pattern, never add a dependency on another craft. Standards shared across the Crafting Apps live in `../craftrules` (not public; ask a maintainer). Read this file first, then `docs/`.

## 1. Orientation (5 minutes)

| Read | Why |
|---|---|
| `docs/architecture.md` | Crate map, layers, data flow (catalog → feed → plan → download → verify → install → inventory), install layout per OS |
| `docs/release-contract.md` | What every craft publishes, what the toolbox relies on, measured state, known gotchas |
| `docs/development.md` | Build, run, test, offline feeds, offscreen UI snapshots, env vars |
| `docs/contributing.md` | Rules, and the "add a command" checklist |
| `docs/roadmap.md` | Milestones M0–M8 and the **current focus** |
| `docs/ui-design.md` | Layout, tokens (dark and light), widgets, accessibility, and the planned behaviour |
| `docs/localization.md` | The 15 UI languages, `tl!`, catalogs, adding a string or a language |
| `SECURITY.md` | Threat model: this app downloads executables and runs them |

## 2. Workspace map

```text
crates/
  release      L0 standalone  the release contract: Version, asset names, Target, SHA256SUMS (pure)
  catalog      L0 standalone  the Crafting Apps, as data (catalog.toml, embedded)
  model        L1             toolbox state: Inventory (installed versions), Settings (serde, no I/O)
  feed         L2             GitHub release JSON -> Release; each app's update Status (pure)
  net          L3             HTTPS (ureq + rustls): GitHub-only host policy per redirect hop, token scoping, caps
  store        L3             the toolbox's files: data dir, atomic writes, settings, inventory, feed cache
  install      L3             per-OS place / activate / remove versions, launch, platform signatures (trust): DMG -> .app, portable zip, AppImage; Layout
  jobs         L4  planned    background work outside the engine (self-update)
  engine       L5             Session + command registry (every action is a command) + background jobs (checks, icons, installs)
  ui-egui      L6             egui shell (thin: all actions go through the engine); i18n catalogs, wording, themes
  automation   L6  planned    control channel + MCP server over the command registry
  testkit          planned    shared test helpers (fake feeds, temp install roots); dev-dependency only
apps/
  artcraft-toolbox            desktop app (eframe/wgpu): window, menu-bar/tray icon, OS notifications, logger
  artcraft-toolbox-cli        headless CLI: list / status / check / install / update / rollback / versions / adopt / uninstall / open / commands / run
xtask/                        cargo xtask layers | ci | contract | version
```

**Layering is enforced** by `cargo xtask layers`. A crate may depend only on lower layers. `release` and `catalog` depend on nothing in the workspace. Nothing below `ui-egui` may use egui, eframe, winit or rfd. **Nothing below L3 may use network, async or archive crates** (ureq, reqwest, tokio, zip, ...): L0–L2 are pure and are tested from bytes. A new crate must be registered in `xtask/src/layers.rs`; the planned ones above are already there, with their layer.

## 3. Golden rules

### Never crash (outranks feature work)

A toolbox that crashes mid-update can leave an app half-installed. A malformed release feed, a hostile asset name, a corrupt settings or inventory file, a full disk or a bad command param must produce an error the user or agent can act on, never a panic. Don't ship a feature by adding a panic path; fix a crash before building on top of it.

- **Non-test code never panics.** No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!`. Return the crate's error type and propagate with `?`; use `ok_or(..)?`, `let .. else { return Err(..) }`, `if let`, or `unwrap_or*` where a fallback is truly correct. Unfinished features return an "unsupported" error (or aren't registered at all). The only exception is a provably infallible literal: `#[allow(clippy::expect_used)]` plus `.expect("why it can't fail")`.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`. If platform interop ever needs it (it hasn't in PhotoCraft's shell), it goes in one isolated crate, reviewed, like PhotoCraft's `tablet`.
- **Input is hostile.** GitHub responses, asset names, checksum files, archives and command params are untrusted. Use `get()` rather than `[i]`; slice strings only at char boundaries; `checked_*`/`saturating_*` for sizes; cap every allocation and loop sized by input (see the `MAX_*` constants in `release`, `catalog`, `model` and `feed`); echo at most a short excerpt of bad input in errors.
- **Don't cascade.** Handle lock poisoning (`lock().unwrap_or_else(PoisonError::into_inner)`) and treat thread joins as `Result`s.
- **Last-resort guard.** `Session::execute` catches a panic that escapes a command and returns `EngineError::Internal`; the app's panic hook logs it. It's a safety net, not a licence to panic. Keep `panic = "unwind"`.
- **Prove it.** Every crash fix comes with a small regression test that panicked before the fix.
- **Enforced by clippy.** `clippy.toml` allows `unwrap`/`expect`/`panic`/indexing in tests only. Every crate carries `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`; new crates start with it.

### Never install what you can't verify (outranks convenience)

The toolbox downloads executables and runs them; a mistake here is a supply-chain hole on the user's machine (`SECURITY.md`).

- **Downloads come only from GitHub release URLs** (`feed::github::DOWNLOAD_PREFIX`, `https://github.com/`, plus GitHub's own redirect to its asset CDN). A feed can never point the toolbox at another host. HTTPS only, no downgrade.
- **Verify before use.** Every downloaded file is checked against its release's `SHA256SUMS.txt` before it is opened, mounted, extracted or run. No checksum entry means no install; a mismatch deletes the file, reports it, and keeps the current version.
- **A checksum is integrity, not authorship.** It comes from the same release as the file. The platform signature is checked too, before a version is activated (`install::trust`: macOS `codesign --verify --deep` and Gatekeeper, Windows Authenticode): broken is refused, and an update must be signed by the same developer as the version it replaces.
- **Archives and disk images are hostile.** Reject zip-slip (`..`, absolute paths, drive letters), symlinks leaving the target, and decompression bombs (cap total size and entry count).
- **Install atomically.** Stage next to the target, then rename into place. Never delete or overwrite the working version before the new one is verified and in place; keep the previous version per `Settings::keep_previous` for rollback.
- **Never elevate silently.** Default installs are per-user and need no admin rights. Anything that would (a per-machine MSI) is an explicit user choice.

### Numbered rules

1. **Everything is a command.** New user-visible behaviour = a command in the engine (`crates/engine/src/<area>_cmds.rs` with a `specs()` function, registered in `commands.rs`): id `<area>.<verb>` (`app.install`), label, params doc, `enabled`, `run`, plus tests. The UI, the CLI and (later) the control channel and MCP all dispatch commands by id. Only pure view state (open tab, search text) lives in `ui-egui/src/state.rs`.
2. **The catalog is data.** Apps, their repos and former names live in `crates/catalog/catalog.toml`. Never hard-code a craft's name, repo or asset naming in code, and never special-case one craft in logic. When a craft deviates, generalise the contract (like `former_slugs`), document it and add a fixture.
3. **The release contract is our API.** `docs/release-contract.md` is everything we rely on from the crafts; `cargo xtask contract` checks it against the live releases. When a craft changes its release pipeline, update the doc, the parser and a fixture in one change.
4. **The core is pure.** L0–L2 take bytes and return data: no network, no filesystem, no clock (pass `now` in), no environment. That is what lets them be tested with real captured responses (`crates/feed/tests/fixtures/`). Above L2, network goes through a `net::Transport` and time through the session's clock (`Session::set_clock`), so tests fake both.
5. **Tests are the gate.** Every change comes with tests. Parsers get hostile-input tests; `engine/tests/panic_hunt.rs` runs every command with adversarial params and must stay green. Network and install code is tested against local fixtures, temp dirs and a local server, never live GitHub in `cargo test` (live checks belong to `cargo xtask contract`).
6. **The UI is thin and data-driven.** Colours and radii come from `theme::Tokens`, never hard-coded, and a new text colour pairing must pass the WCAG AA test in both themes. Rows come from `Session::statuses()`; actions call `ToolboxApp::run(id, params)`. **Every user-facing string is translated:** `tl!("literal")`, `fmt`/`tn` for placeholders and counts, `wording` for text built from engine data; add its translation to all 14 catalogs in the same change (`docs/localization.md`). The engine, CLI and logs stay English.
7. **Verify UI changes visually.** Render offscreen with `cargo run -p artcraft-toolbox-ui-egui --example snapshot` (no window, no focus stealing) and look at the PNG, at the default size and a narrow one, and with `--theme light` and `--lang de` when you touched layout or wording. **No missing-glyph boxes:** egui's default fonts lack many symbols (`→ ✓ ⟳ ⬇`); `status_text_has_no_missing_glyphs` guards status lines, so add new user-facing symbols to it.
8. **Respect the user's machine.** Per-user locations by default; touch nothing outside the install root and the toolbox's data dir; uninstall removes exactly what we installed. **Keep the user's app data across updates**: the crafts' Windows portable zips ship `portable.txt`, which keeps data beside the exe, so a per-version install directory would lose it (`docs/release-contract.md` › Gotchas), so the installer deletes it. **Tests never install into real folders:** give the session a temp `Layout` (`Session::set_layout`); `Session::open` has none, so installing stays off until one is set (the apps call `use_platform_layout`).
9. **Be a good API citizen.** A generic User-Agent (`ArtCraft-Toolbox/<version>`; never a person's name, email or other personal details in requests), conditional requests (ETag), cached feeds, no re-request of an app checked in the last minute, and back-off on GitHub's rate limit. Anonymous users get 60 requests per hour and a `304` still costs one (docs/release-contract.md › GitHub API): every new request path must count against that budget.
10. **Long work is a background job.** Give the command a `start` hook (`engine/src/jobs.rs`): workers send messages, `Session::poll_jobs` applies them on the session's thread. Workers never touch the session, catch their own panics, and stop when cancelled.
11. **Platform features come through `Services`.** The tray, notifications (and later file managers, launching) live in the desktop app crate; `ui-egui` receives them as optional callbacks (`ui_egui::Services`), so tests and the snapshot example run without them. Work that must happen while the window is hidden goes in `ToolboxApp::tick`, which eframe's `App::logic` runs even then.

## 4. Picking work

1. **`docs/roadmap.md` → Current focus**, then that milestone's unchecked items.
2. `log/devlog.md` → the "Still open" bullets of recent entries.
3. `cargo xtask contract` failing? That is the most urgent work there is: a craft changed how it publishes and the toolbox can't follow.

## 5. Before you finish a task

```sh
cargo fmt --all
cargo test -p <crates you touched>
cargo clippy --workspace --all-targets -- -D warnings
cargo xtask layers
cargo xtask ci                 # all of the above in one go (fmt check, clippy, test, layers)
cargo xtask contract           # if you touched release/feed/catalog parsing or catalog.toml (needs network)
cargo run -p artcraft-toolbox-ui-egui --example snapshot -- --out target/snapshots/x.png   # if you touched the UI: look at it
```

New commands come with a graceful-failure test (missing, wrong-type and out-of-range params return `Err`) and are covered by `panic_hunt` automatically.

Then append a terse entry to `log/devlog.md` (local, gitignored: what landed, numbers, what's still open). Sessions can end abruptly, so the dev log plus a green tree is how the next agent picks up. Keep the tree building at every step, and tick the roadmap checklist item you finished.

## 6. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target/agent-<name>`) to avoid the Cargo build lock, and edit only the files you own. Shared files (`engine/src/lib.rs`, the `v.extend(...)` list in `engine/src/commands.rs`, `ui-egui/src/lib.rs`, `state.rs`, `catalog.toml`) get small, surgical edits; re-read before editing.
- Put new commands in a **new module** (`engine/src/<area>_cmds.rs` with `specs()`) rather than growing a shared file.
- If someone else's in-progress edit breaks the build, wait and retry; don't fix their files.
- Keep every `Cargo.toml` valid at all times: the `crates/*` glob means one missing or broken manifest breaks everyone's build. **Create a crate's directory and its `Cargo.toml` together**, and rewrite manifests atomically (write a temp file outside `crates/`, then `mv`).
- Disk space: delete `target/agent-*` dirs of finished agents.

## 7. Where things are tracked

- `docs/roadmap.md`: milestones, their checklists, the current focus.
- `docs/release-contract.md`: the dated, measured state of every craft's releases.
- `plan/` (local, gitignored): research, plans, estimates.
- `log/` (local, gitignored): the dev log.

## 8. Adding a Crafting App

1. Add an `[[app]]` entry to `crates/catalog/catalog.toml` (id = the slug in its asset names).
2. `cargo xtask contract --app <id>`. If it passes, you're done: no code changes.
3. If it fails, the craft deviates from the contract. Prefer fixing the craft's release pipeline (it should follow PhotoCraft's, `../craftrules/release/playbook.md`); otherwise generalise the contract as rule 2 says.
4. `cargo test -p artcraft-toolbox-catalog` (the built-in catalog test counts the apps; update it).
