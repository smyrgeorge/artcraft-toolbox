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
cargo run -p artcraft-toolbox-cli -- status             # statuses (no feed loaded: "Not installed")
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
artcraft-toolbox-cli status [--json] [--feed <app>=<releases.json>]...
artcraft-toolbox-cli commands [--json]                   # every engine command and its params
artcraft-toolbox-cli run <command-id> [<json-params>]    # e.g. run app.status '{"app":"photocraft"}'
```

Exit codes: 0 success, 1 a command failed, 2 a usage error. `status --feed` loads a saved GitHub
"list releases" response, so scripts and tests can see real statuses without the network:

```sh
cargo run -p artcraft-toolbox-cli -- status \
  --feed photocraft=crates/feed/tests/fixtures/photocraft-releases.json
```

## Offscreen UI snapshots (no window)

```sh
cargo run -p artcraft-toolbox-ui-egui --example snapshot -- --out target/snapshots/apps.png \
  --size 440x720 --scale 2 --host macos-aarch64 \
  --feed photocraft=crates/feed/tests/fixtures/photocraft-releases.json --installed photocraft=0.3.0
cargo run -p artcraft-toolbox-ui-egui --example snapshot -- --out target/snapshots/settings.png --tab settings
```

`--feed` and `--installed` put the app list in any state (update available, up to date, not
installed); `--host` picks releases for another machine; `--search` filters the list. Look at the
PNG after every UI change (AGENTS.md rule 7). Rendering needs a wgpu adapter: a GPU, or a
software one such as llvmpipe (Linux) or WARP (Windows).

UI tests (`crates/ui-egui/tests/`) use egui_kittest without rendering: they query the
accessibility tree (`get_by_label`) and click, so they run anywhere, including CI.

## Testing strategy

| Layer | How it is tested |
|---|---|
| `release`, `catalog`, `model` | Unit tests with hostile input: malformed, oversized, overflowing, non-UTF-8-boundary strings |
| `feed` | Real GitHub responses captured in `crates/feed/tests/fixtures/` plus synthetic edge cases |
| `engine` | Command tests per module; `tests/panic_hunt.rs` runs every command with adversarial params |
| `ui-egui` | kittest (accessibility tree), the glyph test, and offscreen snapshots you look at |
| apps | CLI integration tests (output, exit codes, the real binary); desktop arg parsing |
| contract | `cargo xtask contract` against live GitHub, daily in CI (`contract.yml`) |

Never call the network from `cargo test`. Code that needs it (M1+) takes its transport as a
parameter, and tests pass a fake.

## Environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG` | The toolbox's own log level (`debug`, `trace`, `off`); other crates log warnings only |
| `ARTCRAFT_TOOLBOX_BUILD_SHA` | Commit baked into `--version` and About (set by CI and packaging) |
| `ARTCRAFT_TOOLBOX_BUILD_DATE` | Build date baked into `--version` and About |
| `GITHUB_TOKEN` | `cargo xtask contract`: authenticated GitHub API requests (5,000/hour instead of 60) |

Planned (M1): `ARTCRAFT_TOOLBOX_CONFIG_DIR` (data directory override, for tests and agents).

## Logs

The desktop app logs to standard error: `info` for the toolbox's crates, `warn` for everything
else. A log file in the data directory, rotated per launch like PhotoCraft's, comes with the data
directory in M1.

## Versions

The version lives in one place, `[workspace.package] version` in the root `Cargo.toml`:

```sh
cargo xtask version              # print it
cargo xtask version set 0.2.0    # set it (Cargo.toml + Cargo.lock)
```
