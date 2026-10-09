# UI design

ArtCraft Toolbox should feel like a fast suite launcher: a compact panel that drops down from the
menu bar (or pops up from the tray), shows every app with its state, and puts the one useful
action for each app one click away. We study other app managers for *behaviour and layout
grammar* by observation only (clean-room: never copy their code, icons, colours or assets). The
look is PhotoCraft's Studio theme, adapted, so the suite reads as one family.

## Layout

```text
╭────────────────────────────────────────────╮
│ ▣ ArtCraft                              ⟳  │  mark + wordmark; check for updates
│   Toolbox                                  │  (a spinner and "Checking 3 of 12" while it runs)
│ ╭────────────────────────────────────────╮ │
│ │ Apps  Settings                       ⌕ │ │  tabs (underlined), search button
│ ╰────────────────────────────────────────╯ │  (the strip turns into the search field)
│ ╭─ ⚠ error text ……………………………………………… ✕ ─╮ │  banner, while there is an error
│ ╭────────────────────────────────────────╮ │
│ │ Installed                Update all (2)│ │  panel: installed apps first
│ │ ▣• PhotoCraft            [Update]  ⋮   │ │  tile (dot: update waiting) · name · action · menu
│ │    0.5.0 available · 0.3.0 installed   │ │
│ ╰────────────────────────────────────────╯ │
│ ╭────────────────────────────────────────╮ │
│ │ ⌄ Available apps                       │ │  panel, folds
│ │ ▣  VectorCraft                         │ │
│ │    Vector illustration                 │ │  tagline
│ │    [Install] 0.7.0                     │ │  action under it, the version beside
│ ╰────────────────────────────────────────╯ │
│   Checked 5 min ago · 12 apps · macos-…    │  faint footer at the end of the list
╰────────────────────────────────────────────╯
```

- **The window.** On macOS and Windows a popover under (or above) the menu-bar or tray icon,
  440 × 700 points (shorter on a short screen), without a title bar, above other windows and out
  of the taskbar; on macOS a menu-bar app without a Dock icon, and its corners are rounded. A
  click on the icon shows or hides it; it hides when it loses the focus (a click elsewhere,
  launching an app) and on Escape (unless Escape is for the search, a dialog or an open menu);
  Cmd+Q quits. Linux, or a computer whose tray icon couldn't be made: a normal window, 440 × 720
  points, minimum 360 × 480. Placement: `apps/artcraft-toolbox/src/popover.rs`.
- **Click a row** (anywhere but its buttons) to open the app's **details page**: a back arrow,
  the app's card (icon, tagline, status, signature, links to GitHub, Releases, Website, and its
  actions), its own settings (channel, updates, pinned version: combo boxes whose first entry is
  "Default (…)" naming the global value), and its versions, newest first and open, each with its
  release notes rendered from Markdown. The back arrow or the Apps tab returns to the list.
- **One action per row**, chosen by status: Install (not installed), Update (update available;
  filled with the accent: the action that matters), Open (installed and current), Adopt (a copy
  installed outside the toolbox, listed under Installed as "0.3.0 installed outside the toolbox"
  in the warning colour). No action when there is no build for this computer. A spinner replaces
  it while that app is being checked. Update to a version that is kept switches to it at once,
  without a download.
- **Installed rows** put the action on the right with a menu (⋮): Open, Update, Details,
  Uninstall… (asks first, as on the details page). **Available rows** show the app's tagline and
  Install under it, with the version it would install (and "pinned" when a pin caps it).
- **"Update all (N)"** sits on the Installed panel's title line when more than one update waits;
  an app it can't update (running, on macOS) is named in the banner.
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
- **Search**: the search button (or Cmd/Ctrl+F, from any page) turns the tab strip into a search
  field with the focus; the ✕ or Escape closes and clears it. A search unfolds the available apps.
- Names and status lines are truncated with an ellipsis before they reach the buttons (the
  tooltip has the full text). Check at 360 points wide.
- "Check for updates" (the ⟳ button) is disabled with its reason as the tooltip: no network, a
  check running, or GitHub's rate limit (with when it resets); enabled, its tooltip says when the
  last check was.
- Status line colours: accent for "update available", warning for "no build for this computer"
  and copies installed outside the toolbox, danger for a failed check, dim for everything else.
- **Errors** from commands show in a banner above the page (`ToolboxApp::notice`) until the next
  action or its ✕. A check that couldn't reach an app also marks that row ("Couldn't check: …")
  until it succeeds.

### Menu bar and tray

- macOS: a template glyph in the menu bar (`assets/app-icon/tray-template.svg`: only alpha
  counts, macOS tints it). Windows and Linux: the colour icon. With a popover a left click shows
  or hides it and the right button opens the menu; on Linux a left click opens the window.
- Menu: Open ArtCraft Toolbox · Check for Updates · Quit ArtCraft Toolbox.
- Closing the window hides it while there is a tray icon (Settings: "Keep running in the menu
  bar / system tray"); Quit quits.

### Planned behaviour

- On the details page: show in Finder/Explorer.
- A badge on the tray icon when updates are waiting.

## Tokens

Colours and radii come from `theme::Tokens` (`Tokens::get(ctx)`), never hard-coded in a widget.
Two themes, after PhotoCraft's Studio: `Tokens::DARK` (near-black surfaces, a soft violet
accent) and `Tokens::LIGHT` (light grey behind white panels). The `theme` setting picks one;
`system`, the default, follows the OS and switches with it. Both themes' visuals are installed at
start (`set_visuals_of`), so `Tokens::get` follows whichever egui is drawing.

**Contrast.** Every text colour meets WCAG AA (4.5:1) on each surface it is drawn on, in both
themes, hovered rows included (`text_meets_wcag_aa`). That is why the accent has two tokens:
`accent` is a fill (white text on it), `accent_fg` the accent as text. A new token or a new
pairing gets a line in that test.

| Token | Use |
|---|---|
| `bg` | behind everything: the window, the header |
| `card`, `outline` | panels: the tab strip, each list section, the details and settings cards |
| `card_hover` | a hovered row or button |
| `border` | button outlines, the progress track, the popover's edge |
| `field` | inputs |
| `text`, `text_dim`, `text_faint` | primary, secondary, tertiary text (versions, the footer) |
| `accent`, `accent_text` | fills (Update, the selected tab's underline, progress, focus rings, switches) and the text on them |
| `accent_fg` | the accent as text: "update available", phases, links |
| `success`, `warning`, `danger` | statuses, errors, the banner |
| `radius_sm`, `radius`, `radius_lg` | buttons; rows; panels and the popover |

## Type and icons

- **Inter** (Regular, Medium, SemiBold; `assets/fonts/`, SIL OFL), PhotoCraft's UI font, with
  egui's built-in fonts behind it and the system's CJK fonts loaded on demand. Body 13, buttons
  and tabs Medium 12.5–13, app names SemiBold 14, headings SemiBold 17.
- **Lucide icons** (`assets/icons/`, ISC), embedded and rasterized at their on-screen size
  (`icons.rs`, PhotoCraft's pattern). Never a Unicode symbol for an icon.

## Widgets (`widgets.rs`)

- `app_tile`: the app's own icon with rounded corners (fetched from its repository,
  docs/release-contract.md › Icons), or until it arrives a monogram (`PhotoCraft` → `Ph`) on a
  colour hashed from the id; an optional dot on its corner. No craft logo is bundled.
- `panel` / `card`: a rounded panel (`card` with roomier margins), and `panel_title` inside it
  (`Installed`, `Settings for PhotoCraft`), in sentence case.
- `icon_button`: a 28-point icon button, named for screen readers; its name is its tooltip.
- `tab`: a tab underlined in the accent when selected.
- `toggle_row`: an on/off setting, its label on the left and a switch on the right (a checkbox to
  screen readers).
- `primary_button`: the accent-filled button.

## Text

- **Every user-facing string is translated** (docs/localization.md): `tl!("…")` for a literal,
  `fmt` for placeholders, `tn` for counts, `wording` for statuses and engine messages. Leave room:
  German and Russian labels are often a third longer than English, so check new layouts with
  `--lang de` and `--lang ru` at 360 points.
- Product names exactly as the catalog spells them (`PhotoCraft`, `CADCraft`), in every language.
- **No missing glyphs.** Fonts lack many symbols (`→ ✓ ⟳ ⬇`); they draw as boxes. Use an icon
  (`icons.rs`) or words. `status_text_has_no_missing_glyphs` checks status lines and
  `translations_use_glyphs_the_built_in_fonts_have` every catalog; add new user-facing strings
  with symbols to the first.
- Sentence case for labels, buttons and panel titles ("Check for updates", "Available apps").
- Labels inside a clickable row are not selectable (`Label::selectable(false)`): a selectable
  label takes the click meant for the row. Release notes stay selectable, for copying.
- Text colour comes from the widget visuals, never `Visuals::override_text_color`: the override
  also forces link text (release notes) to the label colour.

## Settings

Three cards. **Updates**: Channel, Check every, Keep previous versions, then switches for
installing updates automatically, notifications and keeping the toolbox running in the menu bar
or tray (each disabled, with the reason as its tooltip, where the platform can't). **Appearance**:
Language (Automatic, naming the language it resolves to, then every language by its native name),
Theme (Same as the system, Dark, Light) and Text size (90–150 %, egui's zoom factor). Cmd/Ctrl +,
−, 0 step the text size and save it; egui's own zoom keys are off so the setting stays the truth.
**About**: version, this computer, the data folder.

## Accessibility

- egui publishes an AccessKit tree, which screen readers (VoiceOver, Narrator, Orca) read. A
  row is one button named "{name}, {status}" ("PhotoCraft, 0.5.0 available"; for an app that
  isn't installed the version it would get, not its tagline); its action and menu are separate
  buttons. Icon buttons carry their names ("Check for updates", "Search apps", "More actions for
  PhotoCraft"). Tiles are decorative. The progress bar reports its value.
- Keyboard: Tab reaches every control and row, Enter or Space activates it, Escape closes the
  search, the uninstall question and the popover. A focused row or button draws a 2-point ring in
  `accent`.
- Text size and contrast as above. UI tests find widgets by the same names a screen reader reads
  (`get_by_label`), so a missing name shows up as a failing test.

## Verify

Every visual change: render with the snapshot example (`docs/development.md`), at the default
size and a narrow one (`--size 360x600`), in each state you touched (`--feed`, `--installed`), in
both themes (`--theme light`), in a long-worded language (`--lang de`) and as the popover
(`--popover`), and look at it. The real popover (placement, hiding on focus loss) can only be
seen in the desktop app. Attach before/after PNGs to the PR.
