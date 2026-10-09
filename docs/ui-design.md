# UI design

ArtCraft Toolbox should feel like a fast suite launcher: a compact, tall window that opens fast,
shows every app with its state, and puts the one useful action for each app one click away. We
study other app managers for *behaviour and layout grammar* by observation only (clean-room:
never copy their code, icons or assets).

## Layout

```text
┌──────────────────────────────────────┐
│ ArtCraft Toolbox        [Apps] Settings │  header: title, tabs (M4: refresh, menu)
├──────────────────────────────────────┤
│ [ Search apps                      ] │
│ INSTALLED · 1                        │  installed first: what you use
│ ┌──┐ PhotoCraft            [Update]  │  tile · name · status line · one action
│ └──┘ 0.5.0 available · 0.3.0 installed│
│ AVAILABLE · 11                       │
│ ┌──┐ VectorCraft           [Install] │
│ └──┘ Vector illustration             │
│ …                                    │
├──────────────────────────────────────┤
│ 12 apps · macos-aarch64   0.1.0 (dev)│  status bar: errors replace it
└──────────────────────────────────────┘
```

- Default window 440 × 720 points, minimum 360 × 480.
- **One action per row**, chosen by status: Install (not installed), Update (update available),
  Open (installed and current). No action when there is no build for this computer.
- Status line colours: accent for "update available", success for "up to date", warning for
  "no build for this computer", dim for everything else.
- Errors from commands replace the status bar text (`ToolboxApp::notice`) until the next action.

### Planned behaviour (M3–M4)

- "Update all" at the top of Installed when more than one update is waiting.
- A per-app "⋯" menu: release notes, other versions (install / roll back), show in
  Finder/Explorer, settings (channel, auto-update), uninstall.
- Progress replaces the action button while a job runs (bar + cancel).
- Menu-bar / tray icon; closing the window keeps the toolbox in the tray; a badge when updates
  are waiting.

## Tokens

Colours and radii come from `theme::Tokens` (`Tokens::get(ctx)`), never hard-coded in a widget.
One dark theme for now (`Tokens::DARK`): charcoal surfaces, PhotoCraft's Spectrum-blue accent
`#378ef0`.

| Token | Use |
|---|---|
| `chrome` | header and status bar |
| `bg` | behind the list |
| `card`, `card_hover`, `border` | app rows |
| `field` | inputs |
| `text`, `text_dim`, `text_faint` | primary, secondary, tertiary text |
| `accent`, `accent_text` | selection, primary actions, "update available" |
| `success`, `warning`, `danger` | statuses and errors |
| `radius_sm`, `radius` | widget and card corners |

## Widgets (`widgets.rs`)

- `app_tile`: the app's tile. Until M4 fetches real icons, a monogram (`PhotoCraft` → `Ph`) on a
  colour hashed from the id: stable, distinct, and no logo bundled.
- `card`: a full-width rounded row.
- `section`: `INSTALLED · 1` headings.

## Text

- Product names exactly as the catalog spells them (`PhotoCraft`, `CADCraft`).
- **No missing glyphs.** egui's default fonts lack many symbols (`→ ✓ ⟳ ⬇`); they draw as boxes.
  Prefer words; draw a symbol with the painter if you need one. `status_text_has_no_missing_glyphs`
  checks status lines; add new user-facing strings with symbols to it.
- Sentence case for labels and buttons ("Check for updates", "Keep previous versions").

## Verify

Every visual change: render with the snapshot example (`docs/development.md`), at the default
size and a narrow one (`--size 360x600`), in each state you touched (`--feed`, `--installed`),
and look at it. Attach before/after PNGs to the PR.
