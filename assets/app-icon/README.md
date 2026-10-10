# App icon

`artcraft-toolbox.svg` is the canonical icon: **a toolbox in the Studio accent violet on the Studio
dark tile** (the UI's own colours, `crates/ui-egui/src/theme.rs`): a rounded body and lid, a
handle, a latch and two feet, with a soft glow behind. Original work under MIT OR Apache-2.0 like
the code. Every rendered size comes from it (`packaging/icons.sh`; needs `resvg`, and `iconutil`
on macOS for the `.icns`); the renders are committed, so packaging never needs those tools, and
`.github/workflows/packaging-lint.yml` checks they are there.

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112`, no border. The macOS
renders pad it onto Apple's 824/1024 icon grid (transparent margin); the Windows and Linux
renders crop 22 units off each side so the toolbox reads at 16–48 px.

## Files

| File | Use | Made by |
|---|---|---|
| `artcraft-toolbox.svg` | the master; also the hicolor scalable icon (`hicolor/scalable/apps/ai.storyteller.toolbox.svg`) and the mark in the window's header (`ui-egui/src/icons.rs`) | by hand |
| `artcraft-toolbox-1024.png` | 1024 px render on the macOS grid | `packaging/icons.sh` |
| `artcraft-toolbox.icns` | macOS bundle icon (`CFBundleIconFile`) | `packaging/icons.sh` (iconutil) |
| `artcraft-toolbox.ico` | Windows icon, 16–256 px, embedded in both `.exe` files by `build.rs`, and the MSI's | `packaging/icons.sh` (`cargo xtask ico`) |
| `hicolor/<n>x<n>/apps/ai.storyteller.toolbox.png` | Linux icon theme, 16–512 px; the 128 px one is where the release contract keeps every craft's icon (`docs/release-contract.md` › Icons) | `packaging/icons.sh` |
| `artcraft-toolbox-256.png` | the window icon (`apps/artcraft-toolbox/src/main.rs`) | `packaging/icons.sh` |
| `tray-64.png` | the Windows and Linux tray icon | `packaging/icons.sh` |
| `tray-template.svg`, `tray-template-44.png` | the macOS menu-bar glyph: a template image (black + alpha; the latch is a hole), its own small master | by hand; `packaging/icons.sh` |
