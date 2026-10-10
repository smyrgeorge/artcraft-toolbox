# App icon

`artcraft-toolbox.svg` is the canonical icon, in the Crafting Apps' engraving style: **a classic
metal toolbox**, three-quarter view, with a riveted lid, a hasp and clasp, an arched handle, a
pressed panel and corner caps on the front, feet and a cast shadow. Ink line work and hatching on
a Paper figure, over a solid colour field, the figure filling the rounded tile: the same drawing
language and palette as PhotoCraft's kitsune and the other crafts' icons. Original work under
MIT OR Apache-2.0 like the code. Every rendered size comes from it (`packaging/icons.sh`; needs
`resvg`, and `iconutil` on macOS for the `.icns`); the renders are committed, so packaging never
needs those tools, and `.github/workflows/packaging-lint.yml` checks they are there.

## Palette

Exactly three colours, two of them the family's:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work, hatching, the feet |
| Paper | `#efe9dc` | the toolbox |
| Steel (the toolbox's own colour) | `#4a6f9b` | the full-bleed field: the one hue no craft uses (measured 2026-10-10 from the twelve icons; FilmCraft has the Studio violet) |

Tone is hatching, not grey: three Ink patterns of different density (`light`, `mid`, `cross`)
shade the lid, the body and the dark side, and a slight displacement filter gives every line the
waver of an etched plate. Changing the field is one hex in the SVG.

## Geometry

A 512-unit tile (`viewBox="0 0 512 512"`), rounded square with `rx=112` (a clip path), no border.
The macOS renders pad it onto Apple's 824/1024 icon grid (transparent margin); the Windows and
Linux renders crop 22 units off each side so the toolbox reads at 16–48 px.

## Files

| File | Use | Made by |
|---|---|---|
| `artcraft-toolbox.svg` | the master; also the hicolor scalable icon (`hicolor/scalable/apps/ai.storyteller.toolbox.svg`), the mark in the window's header (`ui-egui/src/icons.rs`) and the DMG window's (`packaging/macos/dmg/background.svg` links it) | by hand |
| `artcraft-toolbox-1024.png` | 1024 px render on the macOS grid | `packaging/icons.sh` |
| `artcraft-toolbox.icns` | macOS bundle icon (`CFBundleIconFile`) | `packaging/icons.sh` (iconutil) |
| `artcraft-toolbox.ico` | Windows icon, 16–256 px, embedded in both `.exe` files by `build.rs`, and the MSI's | `packaging/icons.sh` (`cargo xtask ico`) |
| `hicolor/<n>x<n>/apps/ai.storyteller.toolbox.png` | Linux icon theme, 16–512 px; the 128 px one is where the release contract keeps every craft's icon (`docs/release-contract.md` › Icons) | `packaging/icons.sh` |
| `artcraft-toolbox-256.png` | the window icon (`apps/artcraft-toolbox/src/main.rs`) | `packaging/icons.sh` |
| `tray-64.png` | the Windows and Linux tray icon | `packaging/icons.sh` |
| `tray-template.svg`, `tray-template-44.png` | the macOS menu-bar glyph: a template image (black + alpha; the latch is a hole), its own small master (a menu-bar glyph is a silhouette, not an engraving) | by hand; `packaging/icons.sh` |
