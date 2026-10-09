# Attribution

Every non-code asset in this repository, with its author, source and license. ArtCraft Toolbox's
own code and original assets are MIT OR Apache-2.0 (see [`LICENSE-MIT`](LICENSE-MIT),
[`LICENSE-APACHE`](LICENSE-APACHE) and [`NOTICE`](NOTICE)). When you add an asset, add a row here
in the same change; third-party assets must be permissively licensed and keep their license file
next to them.

## Bundled in the app

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `assets/app-icon/` (all files) | ArtCraft Toolbox placeholder app icon, its PNG renderings (window, tray) and the menu-bar template glyph | ArtCraft Toolbox contributors | Original work, see [`assets/app-icon/README.md`](assets/app-icon/README.md) | MIT OR Apache-2.0 |
| `crates/ui-egui/src/i18n/*.tsv` | UI translations (cs, de, el, es, fr, id, it, ja, ko, pl, pt-br, ru, zh-hans, zh-hant) | ArtCraft Toolbox contributors; strings shared with PhotoCraft reuse PhotoCraft contributors' translations | Original translations of the toolbox's English labels, and PhotoCraft's catalogs | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |

No Crafting App icon or ArtCraft logo is bundled. The app list shows each craft's own icon,
fetched at run time from that craft's repository (docs/release-contract.md › Icons) and cached
in the user's data folder, and a monogram tile until then.
