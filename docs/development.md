# Development guide

## Prerequisites

- Rust stable (1.95+).
- macOS, Windows or Linux. Linux needs `libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev`.
- `curl` for `cargo xtask contract`; `gh` (GitHub CLI) is handy for refreshing fixtures.
- A sibling PhotoCraft checkout (`../photocraft`): the reference for every convention.

## Build and run

```sh
cargo run --release -p artcraft-toolbox                 # desktop app
cargo run -p artcraft-toolbox-cli -- list               # the catalog
cargo run -p artcraft-toolbox-cli -- check              # check GitHub now (uses the data folder's cache)
cargo run -p artcraft-toolbox-cli -- status             # statuses from the last check, no network
cargo test --workspace                                  # everything
cargo xtask ci                                          # fmt + clippy + tests + layers
cargo xtask layers                                      # dependency layering
cargo xtask contract                                    # live check of every craft's releases (network)
```

The workspace profile builds dependencies at `opt-level 2` and the workspace at `opt-level 1`,
like PhotoCraft, so debug builds of the UI stay responsive.

## Headless CLI

```sh
artcraft-toolbox-cli list [--json]
artcraft-toolbox-cli check [--json] [--app <id>] [--force]   # GitHub, in the background job, waited for
artcraft-toolbox-cli status [--json] [--feed <app>=<releases.json>]...
artcraft-toolbox-cli install <app> [--version <x.y.z>] [--json]   # progress on stderr; check first
artcraft-toolbox-cli update <app> [--version <x.y.z>] [--json]    # keeps the replaced version
artcraft-toolbox-cli update --all [--json]
artcraft-toolbox-cli rollback <app> [--version <x.y.z>] [--json]  # switch to a kept version
artcraft-toolbox-cli versions <app> [--json]                      # installed, kept, signatures
artcraft-toolbox-cli adopt <app> [--json]                         # take over a copy installed by hand
artcraft-toolbox-cli uninstall <app> [--json]
artcraft-toolbox-cli open <app>
artcraft-toolbox-cli commands [--json]                   # every engine command and its params
artcraft-toolbox-cli run <command-id> [<json-params>]    # e.g. run app.status '{"app":"photocraft"}'
```

Exit codes: 0 success, 1 a command failed (or `check` couldn't reach every app), 2 a usage error.
Usage errors are found before anything runs, so they never touch the data folder or the network.
The CLI uses the same data folder as the desktop app: `status` shows the last check's results.
`status --feed` loads a saved GitHub "list releases" response instead, so scripts and tests can
see real statuses without the network:

```sh
cargo run -p artcraft-toolbox-cli -- status \
  --feed photocraft=crates/feed/tests/fixtures/photocraft-releases.json
```

`install` installs for real, into the platform's apps folder (docs/architecture.md § 5). To try
it without touching `~/Applications` (or its Windows and Linux equivalents), point a scratch data
folder's `installDir` at a scratch folder first; Ctrl-C mid-download and run it again to see it
resume:

```sh
export ARTCRAFT_TOOLBOX_CONFIG_DIR=/tmp/tb
artcraft-toolbox-cli run settings.set '{"installDir": "/tmp/tb-apps"}'
artcraft-toolbox-cli check --app photocraft
artcraft-toolbox-cli install photocraft --version 0.3.0
artcraft-toolbox-cli update photocraft          # 0.5.0 in use, 0.3.0 kept
artcraft-toolbox-cli versions photocraft
artcraft-toolbox-cli rollback photocraft        # back to 0.3.0, no download
artcraft-toolbox-cli uninstall photocraft       # both versions
```

On macOS the kept versions live in the data folder (`/tmp/tb/versions.noindex/`). Any command
first looks at the disk: a copy of an app put in the apps folder by hand shows up in `versions`
(and `adopt` takes it over), and an app that updated itself is reported as a `note:`.

On Linux the desktop entry and icon still go to `~/.local/share` (only the apps folder moves).

## Offscreen UI snapshots (no window)

```sh
cargo run -p artcraft-toolbox-ui-egui --example snapshot -- --out target/snapshots/apps.png \
  --size 440x720 --scale 2 --host macos-aarch64 \
  --feed photocraft=crates/feed/tests/fixtures/photocraft-releases.json --installed photocraft=0.3.0
cargo run -p artcraft-toolbox-ui-egui --example snapshot -- --out target/snapshots/settings.png --tab settings
```

`--feed` and `--installed` put the app list in any state (update available, up to date, not
installed); `--host` picks releases for another machine; `--search` filters the list;
`--data-dir <dir>` draws a real data folder's state (copied first, never written; no network);
`--checking` draws a check in progress; `--details <app>` opens an app's page and
`--confirm-uninstall` its uninstall question; `--installing <app>` draws an install stopped at
45% of its download (needs that app's `--feed`; nothing is installed). `--installed` may name
several versions of an app (the last one is in use, the others kept) and `--signed` draws them
signed and notarized; `--found <app>=<version>` draws a copy installed outside the toolbox (use
`--host linux-x86_64`); `--online` draws the network-dependent buttons enabled (requests fail at
once, no automatic check). `--lang <code>` draws it in another language (default `en`, whatever the
machine's), `--theme light|dark` in a theme and `--text-size 90..150` at a text size. The example draws
what the desktop app shows, with notifications and a tray icon available. Fill a scratch data
folder for it with `ARTCRAFT_TOOLBOX_CONFIG_DIR=<dir> artcraft-toolbox-cli check` and
`… run icons.refresh`.

The tray, notifications and the hidden-window behaviour can only be seen in the real app (the
tray needs a running event loop). Run it against a scratch data folder and read its log:

```sh
ARTCRAFT_TOOLBOX_CONFIG_DIR=/tmp/tb RUST_LOG=info cargo run -p artcraft-toolbox
grep -h 'tray\|notified' /tmp/tb/logs/artcraft-toolbox.log
```

Look at the PNG after every UI change (AGENTS.md rule 7). Rendering needs a wgpu adapter: a GPU, or a
software one such as llvmpipe (Linux) or WARP (Windows).

UI tests (`crates/ui-egui/tests/`) use egui_kittest without rendering: they query the
accessibility tree (`get_by_label`) and click, so they run anywhere, including CI. They set
`language` to `en` first (`english()` in each test file): the default, `auto`, would follow the
machine's language and the English labels wouldn't be found.

## Testing strategy

| Layer | How it is tested |
|---|---|
| `release`, `catalog`, `model` | Unit tests with hostile input: malformed, oversized, overflowing, non-UTF-8-boundary strings |
| `feed` | Real GitHub responses captured in `crates/feed/tests/fixtures/` plus synthetic edge cases |
| `net` | URL policy and error classification unit tests; `tests/client.rs` runs the real client against a local HTTP server (redirect checks, token scoping, 304, rate limits, size caps) |
| `store` | Temp folders: round trips, corrupt and oversized files, hostile app ids, atomic writes |
| `install` | Temp folders: hostile zips (zip-slip, drive paths, symlinks, duplicates, bombs), AppImage and desktop entry checks, versions side by side, removal refusing foreign paths; on macOS real DMGs made with `hdiutil` (install, update by swap, roll back, prune, refuse to replace a copy installed by hand); signature-output parsers on every OS, real `codesign` and Authenticode checks where they exist |
| `engine` | Command tests per module (checks, installs and updates run against fake transports: rate limits, 304s, failures, cancel, resume, bad checksums, rollback, pruning, a signer change, adoption); `tests/panic_hunt.rs` runs every command with adversarial params, offline, online and with a scratch install layout |
| `ui-egui` | kittest (accessibility tree): rows, details page, pinning, check button, notifications, close-to-tray, install and uninstall, update all, switching versions, adopting, automatic updates, live language switching, themes, text size, keyboard focus; catalog coverage, placeholders, plurals and glyphs (`i18n/tests.rs`), WCAG contrast; the glyph test; offscreen snapshots you look at. Tests that wait for background work wait by a deadline, not a frame count, and don't tick unless they need to; check them with a slowed fake network before pushing (CI machines are slower) |
| apps | CLI integration tests (output, exit codes, install / uninstall / open against a fake GitHub, the real binary); desktop arg parsing |
| contract | `cargo xtask contract` against live GitHub, daily in CI (`contract.yml`) |

Never call the network from `cargo test`. Code that needs it takes a `net::Transport`; tests pass
a fake (or the real client against a local server, `crates/net/tests/client.rs`). The CLI's
`run(args, out, err, env)` takes its data folder and transport the same way.

Never install into real folders from a test either. A `Session` from `Session::open` has no
install layout (installing is refused); give it a temp one with `Session::set_layout`, and the
CLI's `Env` a `layout`. Only the apps' startup (`setup::open_in`) uses the platform layout.

## Environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG` | The toolbox's own log level (`debug`, `trace`, `off`); other crates log warnings only |
| `ARTCRAFT_TOOLBOX_BUILD_SHA` | Commit baked into `--version` and About (set by CI and packaging) |
| `ARTCRAFT_TOOLBOX_BUILD_DATE` | Build date baked into `--version` and About |
| `ARTCRAFT_TOOLBOX_CONFIG_DIR` | Data folder override (settings, inventory, feed cache, logs); tests and agents use a temp folder |
| `ARTCRAFT_TOOLBOX_LOCALE` | A language tag (`ja`, `pt-BR`) used instead of the system's languages when the `language` setting is `auto` (docs/localization.md) |
| `ARTCRAFT_TOOLBOX_GITHUB_TOKEN` | GitHub token for the apps' checks: 5,000 requests/hour instead of 60; sent only to `api.github.com` |
| `GITHUB_TOKEN` | `cargo xtask contract` only: authenticated GitHub API requests |

## Logs

The desktop app logs to standard error and to `<data dir>/logs/artcraft-toolbox.log` (each
launch moves the previous log to `.1.log`, then `.2.log`; the file stops at 16 MiB). That file is
what a bug report attaches. Defaults: `info` for the toolbox's crates, `warn` for everything else;
`RUST_LOG` takes env_logger-style directives (`debug`, `warn,artcraft_toolbox_net=trace`,
`artcraft_toolbox*=debug`). The logger is PhotoCraft's, ported (`apps/artcraft-toolbox/src/logging.rs`).

## Versions

The version lives in one place, `[workspace.package] version` in the root `Cargo.toml`:

```sh
cargo xtask version              # print it
cargo xtask version set 0.2.0    # set it (Cargo.toml + Cargo.lock)
```
