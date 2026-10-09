# UI design

ArtCraft Toolbox should feel like a fast suite launcher: a compact, tall window that opens fast,
shows every app with its state, and puts the one useful action for each app one click away. We
study other app managers for *behaviour and layout grammar* by observation only (clean-room:
never copy their code, icons or assets).

## Layout

```text
┌────────────────────────────────────────────┐
│ ArtCraft Toolbox           [Apps] Settings │  header: title, tabs (M4: menu)
├────────────────────────────────────────────┤
│ [ Search apps        ] [Check for updates] │  or a spinner and "Checking 3 of 12"
│ INSTALLED · 1                              │  installed first: what you use
│ ┌──┐ PhotoCraft                  [Update]  │  tile · name · status line · one action
│ └──┘ 0.5.0 available · 0.3.0 installed     │
│ AVAILABLE · 11                             │
│ ┌──┐ VectorCraft                 [Install] │
│ └──┘ Vector illustration                   │
│ …                                          │
├────────────────────────────────────────────┤
│ Checked 5 min ago · 12 apps · linux-x86_64 │  status bar: errors replace it
└────────────────────────────────────────────┘
```

- Default window 440 × 720 points, minimum 360 × 480.
- **Click a row** (anywhere but its action) to open the app's **details page**: icon, tagline,
  status, links (GitHub, Releases, Website), the app's own settings (channel, updates, pinned
  version: combo boxes whose first entry is "Default (…)" naming the global value), and its
  versions, newest first and open, each with its release notes rendered from Markdown. "Back"
  or the Apps tab returns to the list.
- **One action per row**, chosen by status: Install (not installed), Open (installed). Update
  (update available) comes with M3; until then an installed app shows Open. No action when there
  is no build for this computer. A spinner replaces it while that app is being checked.
- **Installing** (M2): the row's action becomes Cancel and its status line the phase
  ("Verifying the release", "Downloading 45%", "Verifying", "Installing") in the accent colour,
  with a thin bar under it (a sliding segment while there is no fraction). Cancel keeps the
  partial download, so Install resumes it. Several apps can install at once.
- **The details page's actions**: the row's action, then Update (disabled until M3) and
  Uninstall, which asks first: "Uninstall PhotoCraft 0.5.0?", its Uninstall in the danger colour;
  Cancel, Escape or a click outside keeps the app.
- Names and status lines are truncated with an ellipsis before they reach the action column
  (the tooltip has the full text). Check at 360 points wide.
- "Check for updates" is disabled with its reason as the tooltip: no network, a check running,
  or GitHub's rate limit (with when it resets).
- Status line colours: accent for "update available", success for "up to date", warning for
  "no build for this computer", dim for everything else.
- Errors from commands replace the status bar text (`ToolboxApp::notice`) until the next action.
  A check that couldn't reach an app also marks that row ("Couldn't check: …") until it succeeds.

### Menu bar and tray (M4)

- macOS: a template glyph in the menu bar (`assets/app-icon/tray-template.svg`: only alpha
  counts, macOS tints it); a click opens the menu. Windows and Linux: the colour icon; a left
  click opens the window, the right button the menu.
- Menu: Open ArtCraft Toolbox · Check for Updates · Quit ArtCraft Toolbox.
- Closing the window hides it while there is a tray icon (Settings: "Keep running in the menu
  bar / system tray"); Quit quits.

### Planned behaviour (M3)

- "Update all" at the top of Installed when more than one update is waiting.
- On the details page: install another version (roll back), show in Finder/Explorer.
- A badge on the tray icon when updates are waiting.

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

- `app_tile`: the app's own icon (fetched from its repository, docs/release-contract.md › Icons),
  or until it arrives a monogram (`PhotoCraft` → `Ph`) on a colour hashed from the id. No craft
  logo is bundled.
- `subhead`: a heading without a count (`SETTINGS FOR PHOTOCRAFT`).
- `card`: a full-width rounded row.
- `section`: `INSTALLED · 1` headings.

## Text

- Product names exactly as the catalog spells them (`PhotoCraft`, `CADCraft`).
- **No missing glyphs.** egui's default fonts lack many symbols (`→ ✓ ⟳ ⬇`); they draw as boxes.
  Prefer words; draw a symbol with the painter if you need one. `status_text_has_no_missing_glyphs`
  checks status lines; add new user-facing strings with symbols to it.
- Sentence case for labels and buttons ("Check for updates", "Keep previous versions").
- Labels inside a clickable row are not selectable (`Label::selectable(false)`): a selectable
  label takes the click meant for the row. Release notes stay selectable, for copying.
- Text colour comes from the widget visuals, never `Visuals::override_text_color`: the override
  also forces link text (release notes) to the label colour.

## Verify

Every visual change: render with the snapshot example (`docs/development.md`), at the default
size and a narrow one (`--size 360x600`), in each state you touched (`--feed`, `--installed`),
and look at it. Attach before/after PNGs to the PR.
