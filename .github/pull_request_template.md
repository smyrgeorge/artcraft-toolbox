## What and why

<!-- One or two sentences. Link the roadmap item or issue. -->

## How it was verified

- [ ] `cargo xtask ci` passes (fmt, clippy `-D warnings`, tests, layering)
- [ ] `cargo xtask contract` passes, if release, feed or catalog parsing or `catalog.toml` changed
- [ ] UI changed: snapshot PNGs attached (before / after), no missing-glyph boxes
- [ ] New or changed commands: graceful-failure tests, `docs/architecture.md` § 6 updated
- [ ] Downloads, installs or file handling changed: `SECURITY.md` rules followed, hostile-input tests added
- [ ] `docs/roadmap.md` checklist item ticked
