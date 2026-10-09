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
- **One action per row**, chosen by status: Install (not installed), Update (update available),
  Open (installed and current), Adopt (a copy installed outside the toolbox, listed under
  Installed as "0.3.0 installed outside the toolbox" in the warning colour). No action when there
  is no build for this computer. A spinner replaces it while that app is being checked. Update to
  a version that is kept switches to it at once, without a download.
- **"Update all (N)"** sits on the Installed heading's line when more than one update waits; an
  app it can't update (running, on macOS) is named in the status bar.
- **Installing or updating**: the row's action becomes Cancel and its status line the phase
  ("Verifying the release", "Downloading 45%", "Verifying", "Installing", "Checking the
  signature") in the accent colour, with a thin bar under it (a sliding segment while there is no
  fraction). Cancel keeps the partial download, so the next attempt resumes it. Several apps can
  install at once.
- **The details page's actions**: the row's action, then Open (when the action is Update) and
  Uninstall, which asks first: "Uninstall PhotoCraft 0.5.0?" (saying how many kept versions go
  with it), its Uninstall in the danger colour; Cancel, Escape or a click outside keeps the app.
  Under the status line, the signature: "Signed by Learning Machines LLC (DJ6XS33FX8) ·
  notarized", or "No platform signature".
- **Versions**: each release is marked "in use" or "kept"; opening one offers "Switch to this
  version" (kept: instant, no download) or "Install this version" (any other release with a build
  here: a download, older ones included).
- **Automatic updates** (Settings, or per app): after a check, the apps set to update
  automatically are updated in the background and announced when done ("Updated: PhotoCraft
  0.5.0"); they get no "Update available" notification.
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

### Planned behaviour

- On the details page: show in Finder/Explorer.
- A badge on the tray icon when updates are waiting.

## Tokens

Colours and radii come from `theme::Tokens` (`Tokens::get(ctx)`), never hard-coded in a widget.
Two themes: `Tokens::DARK` (charcoal surfaces, a Spectrum-blue accent) and `Tokens::LIGHT` (light
grey surfaces, white cards, buttons outlined). The `theme` setting picks one; `system`, the
default, follows the OS and switches with it. Both themes' visuals are installed at start
(`set_visuals_of`), so `Tokens::get` follows whichever egui is drawing.

**Contrast.** Every text colour meets WCAG AA (4.5:1) on each surface it is drawn on, in both
themes (`text_meets_wcag_aa`). That is why the accent has two tokens: `accent` is a fill (white
text on it), `accent_fg` the accent as text on `bg` and `card`. A new token or a new pairing gets
a line in that test.

| Token | Use |
|---|---|
| `chrome` | header and status bar |
| `bg` | behind the list |
| `card`, `card_hover`, `border` | app rows |
| `field` | inputs |
| `text`, `text_dim`, `text_faint` | primary, secondary, tertiary text |
| `accent`, `accent_text` | fills (selected tab, primary buttons, progress) and the text on them |
| `accent_fg` | the accent as text: "update available", phases, links |
| `success`, `warning`, `danger` | statuses and errors |
| `radius_sm`, `radius` | widget and card corners |

## Widgets (`widgets.rs`)

- `app_tile`: the app's own icon (fetched from its repository, docs/release-contract.md › Icons),
  or until it arrives a monogram (`PhotoCraft` → `Ph`) on a colour hashed from the id. No craft
  logo is bundled.
- `subhead`: a heading without a count (`SETTINGS FOR PHOTOCRAFT`).
- `card`: a full-width rounded row.
- `section`: `INSTALLED · 1` headings. Headings are upper-cased with `widgets::caps`, which drops
  Greek accents in capitals, as Greek typesetting does.

## Text

- **Every user-facing string is translated** (docs/localization.md): `tl!("…")` for a literal,
  `fmt` for placeholders, `tn` for counts, `wording` for statuses and engine messages. Leave room:
  German and Russian labels are often a third longer than English, so check new layouts with
  `--lang de` and `--lang ru` at 360 points.
- Product names exactly as the catalog spells them (`PhotoCraft`, `CADCraft`), in every language.
- **No missing glyphs.** egui's default fonts lack many symbols (`→ ✓ ⟳ ⬇`); they draw as boxes.
  Prefer words; draw a symbol with the painter if you need one. `status_text_has_no_missing_glyphs`
  checks status lines; add new user-facing strings with symbols to it.
- Sentence case for labels and buttons ("Check for updates", "Keep previous versions").
- Labels inside a clickable row are not selectable (`Label::selectable(false)`): a selectable
  label takes the click meant for the row. Release notes stay selectable, for copying.
- Text colour comes from the widget visuals, never `Visuals::override_text_color`: the override
  also forces link text (release notes) to the label colour.

## Settings › Appearance

Language (Automatic, naming the language it resolves to, then every language by its native name),
Theme (Same as the system, Dark, Light) and Text size (90–150 %, egui's zoom factor). Cmd/Ctrl +,
−, 0 step the text size and save it; egui's own zoom keys are off so the setting stays the truth.

## Accessibility

- egui publishes an AccessKit tree, which screen readers (VoiceOver, Narrator, Orca) read. A
  row is one button named "{name}, {status}" ("PhotoCraft, 0.5.0 available"); its action is a
  separate button. Tiles are decorative. The progress bar reports its value.
- Keyboard: Tab reaches every control and row, Enter or Space activates it, Escape closes the
  uninstall question. A focused row draws a 2-point ring in `accent`.
- Text size and contrast as above. UI tests find widgets by the same names a screen reader reads
  (`get_by_label`), so a missing name shows up as a failing test.

## Verify

Every visual change: render with the snapshot example (`docs/development.md`), at the default
size and a narrow one (`--size 360x600`), in each state you touched (`--feed`, `--installed`), in
both themes (`--theme light`) and in a long-worded language (`--lang de`), and look at it. Attach before/after PNGs to the PR.
