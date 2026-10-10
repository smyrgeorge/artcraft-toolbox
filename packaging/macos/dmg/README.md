# macOS DMG window

What Finder shows when the DMG opens: a 660 × 400 pt background with the app icon and the
`Applications` link side by side (PhotoCraft's `packaging/macos/dmg/`, ported). `package.sh`
copies these files into the image; nothing here is generated at build time, so the DMG still
builds with `hdiutil makehybrid` (no mounted device, no Finder scripting on CI, nothing extra
installed in the signing job).

| File | What |
|---|---|
| `background.svg` | Source of the background: a band in the icon's steel field (`#4a6f9b`) with the app icon (`assets/app-icon/artcraft-toolbox.svg`, linked, not copied) on the left, the Paper field (`#efe9dc`) under the icons, and an arrow from the app to `Applications` in the same steel. No text, so rendering it needs no fonts. |
| `background.tiff` | The background at 1x (660 × 400 px, 72 dpi) and 2x (1320 × 800 px, 144 dpi) in one HiDPI TIFF (16-colour palette, Deflate, sRGB). Goes to `.background/background.tiff`. |
| `dmg-layout.DS_Store` | Finder's view settings for the volume: window size, icon size 128, `ArtCraft Toolbox.app` at (326, 205), `Applications` at (574, 205), and the background. Goes to `.DS_Store` in the image; named so it isn't mistaken for (or ignored like) a Finder-generated `.DS_Store`. |
| `generate.py` | Writes `background.tiff` and `dmg-layout.DS_Store` from the SVG and the layout above. |

## The volume name has no version

The mounted volume is called `ArtCraft Toolbox`, not `ArtCraft Toolbox <version>`. The layout
points at the background through an alias that includes the volume name, and Finder resolves it
by that name: with a versioned name the window keeps its size and icon positions but shows no
background. The DMG file name (`artcraft-toolbox-<version>-macos-<arch>.dmg`) still carries the
version. (The toolbox itself never mounts a craft's image at `/Volumes/<Name>`: it uses a private
mount point, release contract › Gotchas 7.)

## Rules

- **Finder draws the icon labels in black in light and dark mode** when a window has a background,
  so the area under both icons stays light (Paper).
- **Nothing goes inside the icon boxes:** artwork keeps 10 pt clear of each 128 pt icon box and of
  the label strip under it.

## Regenerate

Edit `background.svg` (or the layout constants in `generate.py`), then run, on any OS:

```sh
pip install 'pillow>=12' ds_store==1.3.3 mac_alias==2.2.3
python3 packaging/macos/dmg/generate.py   # needs resvg on PATH
```

Both outputs are byte-for-byte reproducible and carry nothing from the machine that ran it: the
SVG needs no fonts, and `dmg-layout.DS_Store` is written from scratch, its background alias
holding only the volume name and `/.background/background.tiff`. If you ever add text to the
SVG, outline it (`usvg` from resvg converts text to paths). The window is 660 × 432 with Finder's
32 pt title bar; the content area is 660 × 400.
