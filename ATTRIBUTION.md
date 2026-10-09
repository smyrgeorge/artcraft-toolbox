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
| `assets/fonts/Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter 4.001 (UI font), as PhotoCraft bundles it | The Inter Project Authors (Rasmus Andersson) | <https://github.com/rsms/inter> | SIL OFL 1.1, [`assets/fonts/OFL-Inter.txt`](assets/fonts/OFL-Inter.txt) |
| `assets/icons/*.svg` | Lucide icons (arrow-left, check, chevron-down, chevron-right, download, ellipsis-vertical, external-link, folder-open, info, refresh-cw, search, settings, trash, triangle-alert, x) | Lucide Icons and Contributors | <https://github.com/lucide-icons/lucide> (`icons/<name>.svg`, release 1.54.0, or PhotoCraft's copies) | ISC, [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `assets/icons/{arrow-left,check,chevron-down,chevron-right,download,external-link,info,search,trash,x}.svg` | Lucide icons derived from Feather (subset of the row above) | Cole Bemis (Feather), Lucide Contributors | <https://github.com/feathericons/feather> via Lucide | MIT (Feather) and ISC (Lucide), [`assets/icons/LICENSE-lucide.txt`](assets/icons/LICENSE-lucide.txt) |
| `crates/ui-egui/src/i18n/*.tsv` | UI translations (cs, de, el, es, fr, id, it, ja, ko, pl, pt-br, ru, zh-hans, zh-hant) | ArtCraft Toolbox contributors; strings shared with PhotoCraft reuse PhotoCraft contributors' translations | Original translations of the toolbox's English labels, and PhotoCraft's catalogs | MIT OR Apache-2.0, [`LICENSE-translations.txt`](crates/ui-egui/src/i18n/LICENSE-translations.txt) |

## In the documentation

| Path | Title | Author | Source | License |
|---|---|---|---|---|
| `docs/images/toolbox-*.png` | Screenshots of ArtCraft Toolbox, rendered offscreen by `crates/ui-egui/examples/snapshot.rs` | ArtCraft Toolbox contributors | Original | MIT OR Apache-2.0; the Crafting Apps' icons shown in them are those apps' own artwork, from their repositories (`storytold/<app>`), shown as the toolbox displays them |

No Crafting App icon or ArtCraft logo is bundled. The app list shows each craft's own icon,
fetched at run time from that craft's repository (docs/release-contract.md › Icons) and cached
in the user's data folder, and a monogram tile until then.
