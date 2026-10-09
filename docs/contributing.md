# Contributing

- **Language:** Rust only. No JavaScript or TypeScript, no webview.
- **Licence:** contributions are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The ArtCraft name and logos are the ArtCraft Team's trademarks and are not bundled.
- **Clean-room:** we study other app managers for behaviour and look only. Don't copy code, icons or other assets from them. Third-party assets must be permissively licensed, keep their license next to the asset, and get a row in `ATTRIBUTION.md` in the same change.
- **Never crash:** non-test code must not panic and must not use `unsafe`. See *Never crash* in `AGENTS.md`.
- **Never install what you can't verify:** see the rules in `AGENTS.md` and `SECURITY.md`.
- **Commands, not handlers:** new features are engine commands with tests, and the UI calls them (checklist below).
- **Layering:** `cargo xtask layers` must pass. Register new crates in `xtask/src/layers.rs`.
- **Tests:** required for every change. Parsers need malformed-input tests; anything that touches the network or the disk is tested with fixtures and temp dirs, not live services.
- **Style:** `cargo fmt`, and `cargo clippy --workspace --all-targets -- -D warnings`. Match surrounding code. Comments explain *why*.
- **UI:** use `theme::Tokens` and `widgets::*`. Verify visually (the offscreen `snapshot` example) and attach before/after screenshots to PRs.
- **Strings:** user-facing text goes through `tl!` (or `wording`) and gets a translation in every catalog in the same change; see `docs/localization.md`.
- **Commits:** small, focused, with a clear subject line.

## Adding a command

1. **Pick the id**: `<area>.<verb>` (`app.install`, `apps.updateAll`). One area per module.
2. **Logic** goes in the lowest crate that fits: parsing and decisions in `release`, `catalog`, `model` or `feed` (pure, unit-tested); disk and network in `net`/`install` (L3); multi-step work in `jobs` (L4).
3. **Command** goes in an engine module (`crates/engine/src/<area>_cmds.rs`) exposing `specs()`, registered with `v.extend(...)` in `commands.rs`. Fill in:
   - `id` and `label`,
   - a params doc such as `{"app":"<id>","version"?:"x.y.z"}` (what agents read through `commands`),
   - an `enabled` predicate (greys the action out),
   - `run`, which validates params with the helpers in `params.rs` (`only`, `str`, `app`) and returns a JSON result.
4. **Tests** in the module: behaviour, disabled states, and bad params. `tests/panic_hunt.rs` covers every registered command automatically; add a case to its `adversarial()` list if your params have a new shape.
5. **UI**: call it through `ToolboxApp::run(id, params)`; errors land in the status bar.
6. **Docs**: add it to the command table in `docs/architecture.md` § 6.
