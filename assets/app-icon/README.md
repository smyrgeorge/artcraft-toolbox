# App icon

`artcraft-toolbox.svg` is a **placeholder** (a toolbox in the accent violet on the dark tile), original
work under MIT OR Apache-2.0 like the code.

| File | Use | Made with |
|---|---|---|
| `artcraft-toolbox-256.png` | window icon | `resvg -w 256 -h 256 artcraft-toolbox.svg artcraft-toolbox-256.png` |
| `tray-64.png` | Windows and Linux tray icon | `resvg -w 64 -h 64 artcraft-toolbox.svg tray-64.png` |
| `tray-template.svg`, `tray-template-44.png` | macOS menu-bar glyph: a template image (black + alpha; the latch is a hole) | `resvg -w 44 -h 44 tray-template.svg tray-template-44.png` |

The real icon and its rendered sizes (macOS `.icns`, Windows `.ico`, Linux hicolor PNGs) come with
packaging in roadmap M5, generated the way PhotoCraft does it (`packaging/icons.sh`: one 512-unit
SVG master, rendered with resvg).
