# Localization

The desktop app's interface is translated; the CLI, engine errors from the network or the disk,
command ids and logs stay in English. The pattern is PhotoCraft's (`../photocraft/docs/localization.md`),
ported: English source strings are the lookup keys, UTF-8 TSV catalogs supply the translations,
and anything missing falls back to English.

## Languages

| Language | Code | Plural forms |
|---|---|---|
| English | `en` | one / other |
| 日本語 | `ja` | one form |
| 简体中文 | `zh-hans` | one form |
| 繁體中文 | `zh-hant` | one form |
| Español | `es` | one / other |
| Русский | `ru` | one / few / many |
| Čeština | `cs` | one / few / other |
| Français | `fr` | singular for 0 and 1 / plural for 2+ |
| Bahasa Indonesia | `id` | one form |
| 한국어 | `ko` | one form |
| Polski | `pl` | one / few (2–4, not 12–14) / many |
| Deutsch | `de` | one / other |
| Português (Brasil) | `pt-br` | singular for 0 and 1 / plural for 2+ |
| Ελληνικά | `el` | one / other |
| Italiano | `it` | one / other |

Every catalog covers every `tl!` string in `ui-egui`, every plural message, the catalog's
taglines and the engine's fixed messages the UI shows (`wording::ENGINE_STRINGS`: disabled
reasons, job phases). The tests insist on it.

## Choosing the language

Settings › Appearance › Language, or `settings.set {"language": "<code>"}` from the CLI or an
agent. The default, `auto`, takes the first supported language from the system's preferred UI
languages (`sys-locale`: `CFLocaleCopyPreferredLanguages` on macOS,
`GetUserPreferredUILanguages` on Windows, the locale environment on Linux), else English.
Regional tags resolve to their catalog (`de-AT` → `de`, `zh-TW` → `zh-hant`, every `pt` → `pt-br`).
`ARTCRAFT_TOOLBOX_LOCALE=<tag>` overrides the system for one launch.

A change applies on the next frame, without a restart: the window, the tray menu (rebuilt) and
later notifications. Nothing about an app's state depends on the language.

## In the code

```rust
ui.button(tl!("Check for updates"));                                   // a literal
fmt(tl!("Update {app} to {version}"), &[("app", &name), ("version", &v)]);  // placeholders
tn(count, "{n} app", "{n} apps");                                       // plurals ({n} filled in)
```

- `tl!` takes only a string literal; a test scans the sources for them, so a string built at run
  time is never seen. Text that comes from elsewhere goes through `wording`
  (`wording::status`, `wording::trust`, …) or, for a fixed engine message, `ENGINE_STRINGS`.
- App names, version numbers and paths are never translated. Taglines come from `catalog.toml`
  in English and are translated at display (`wording::tagline`); a new craft's tagline needs its
  translations too.
- Headings use `widgets::caps`, which drops Greek accents in capitals (`ΡΥΘΜΙΣΕΙΣ`, not
  `ΡΥΘΜΊΣΕΙΣ`), as Greek typesetting does.
- UI tests pin `language` to `en` (this repo's tests must pass on any machine's language).

## Fonts

egui's built-in fonts cover the Latin, Greek and Cyrillic catalogs
(`translations_use_glyphs_the_built_in_fonts_have`). Chinese, Japanese and Korean come from the
system's fonts, loaded on demand (`cjk_fonts`, PhotoCraft's loader): a CJK UI language puts its
script's fonts first. No font is bundled.

## Adding or changing a language

1. Add `<code>.tsv` under `crates/ui-egui/src/i18n/`, with the header of `de.tsv`.
2. Register its code, native name and plural rule in `i18n::LANGUAGES`.
3. Translate the meaning of each English string. Keep `{name}` placeholders, escapes and the
   trailing `…`; give each `@plural` entry as many forms as the language's rule returns. Product
   names (ArtCraft Toolbox, PhotoCraft, GitHub, macOS) stay as they are. Strings shared with
   PhotoCraft may reuse its wording.
4. Run `cargo test -p artcraft-toolbox-ui-egui` and render the snapshot example with
   `--lang <code>`, at 440 and 360 points wide; look for truncated labels and missing glyphs.

Adding an English string means adding its translation to every catalog in the same change
(`every_string_is_translated_in_every_language` lists what's missing). Translations are original
work or reuse PhotoCraft's for the same English text (MIT OR Apache-2.0); no proprietary
localization resource is used (`crates/ui-egui/src/i18n/LICENSE-translations.txt`).
