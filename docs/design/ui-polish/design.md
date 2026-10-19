# UI polish — Design

**Spec:** [spec.md](spec.md)   **Status:** Reviewed (design gate passed after 2 review passes)   **Last updated:** 2026-10-19

## Overview
This is a frontend-only overhaul in four layers:

1. **Foundations:** bundled fonts, contrast-safe tokens, a 12 px type floor, and a font reset for form controls.
2. **Shared building blocks:** button classes and a small set of behavioural components in `src/lib/ui/`, plus a shared chart theme.
3. **Shell:** a labelled navigation, one Esc routing rule, real OS fullscreen, and page width containers.
4. **Screens:** each screen is rebuilt on those layers. The ride screen gets a viewport-fitted layout whose numbers scale with the space available.

A mocked-backend Playwright harness moves into the repo at `tools/ui-harness/`. It produces the evidence for the spec's ACs and success signals.

## Platform constraints
- **Webviews:** GNOME uses WebKitGTK and macOS uses WKWebView. Both are WebKit, so WebKit results are authoritative and Chromium is a second opinion.
- **macOS minimum:** Tauri's default floor is macOS 10.13 and `minimumSystemVersion` isn't set. So we avoid CSS features that are only newer than Safari 15.4:
  - no container queries or `cq*` units;
  - no `:has()` for layout.
  - `<dialog>` is already in use (Safari 15.4+), and that is our floor.
  - Sizing that depends on the available space is done with a `ResizeObserver` that writes CSS custom properties.

## Architecture decisions

### ADR-1: System UI font plus a bundled monospace font for data
- **Context:** Outfit and IBM Plex Mono are loaded from Google Fonts. This blocks render: the UI is blank for about 28 s on a hanging network, and the app contacts Google on every launch. The user chose "system font per OS + bundled fonts".
- **Options:**
  - (a) self-host Outfit and Plex Mono;
  - (b) system UI font for interface text, with Plex Mono bundled for numbers;
  - (c) system fonts for everything.
- **Decision:** (b).
  - `--font-sans: system-ui, -apple-system, BlinkMacSystemFont, 'Cantarell', 'Adwaita Sans', 'Noto Sans', 'Segoe UI', sans-serif`. This gives SF Pro on macOS and the GNOME default (Adwaita Sans or Cantarell) on Linux.
  - `--font-data: 'IBM Plex Mono', ui-monospace, 'SF Mono', monospace`, using `@fontsource/ibm-plex-mono` (OFL, pinned exact). Weights 400, 500, 600 and 700, Latin subset, imported once in `+layout.svelte`.
  - Remove the Google `<link>`/preconnects from `app.html`, and the `fonts.googleapis.com`/`fonts.gstatic.com` sources from the CSP.
- **Why:** this feels native on each OS, keeps the distinctive tabular numerals for the data, and makes no network requests.
- **Consequence:** the brand loses Outfit's geometric feel, which the user accepted (Q1). Headings use `font-weight: 700` and a tighter letter-spacing so the hierarchy still reads.

### ADR-2: Global button classes plus a few behavioural components
- **Context:** there are about 15 per-file button styles and two clashing `.btn-secondary` definitions. Svelte 5 components that wrap `<button>` add prop-forwarding friction across about 75 call sites.
- **Decision:**
  - **Buttons** are global classes in `src/lib/ui/controls.css`, imported by `app.css`:
    - `.btn` (36 px tall);
    - `.btn-lg` (44 px);
    - variants `.btn-primary`, `.btn-secondary`, `.btn-ghost`, `.btn-danger` and `.btn-success`;
    - `.btn-icon` (square).
  - **Components** cover only the pieces that carry behaviour or structure: `IconButton`, `SegmentedControl`, `Dialog`, `Notice`, `StateView`, `Switch`, all under `src/lib/ui/`.
- **Why:** the least churn for plain buttons, and one implementation of the tricky parts: tooltip on focus, radiogroup semantics, the dialog's Esc handling and close button, and retry and details in error states.

### ADR-3: Ride screen sized by measuring the space it has
- **Context:** we need numbers that grow with the window (AC12) and controls that are always visible (AC11), without container queries.
- **Options:**
  - (a) `vw`/`vh` clamps;
  - (b) a `ResizeObserver` on the ride stage writing `--stage-w`/`--stage-h`, plus a layout class from a pure function;
  - (c) scaling the whole stage with a CSS transform.
- **Decision:** (b).
  - **Page structure:** the ride page is a full-height flex column with three parts:
    1. the header, auto height;
    2. the stage, `flex: 1; min-height: 0; overflow: hidden`;
    3. the control bar, auto height and never shrinking.
  - **Layout function:** `rideLayout(w, h)` in `src/lib/utils/rideLayout.ts` returns:
    - `{ mode: 'wide' | 'stacked', powerPx, valuePx, smallValuePx, showSecondary }`;
    - `wide` when `w / h ≥ 1.45`;
    - font sizes as linear functions of `min(w / cols, h / rows)`, clamped to the minimums in AC12;
    - `smallValuePx` has a floor of 28, so its 0.45em unit stays ≥ 12.6 px.
    - `showSecondary: false` when the stage can't fit the secondary strip at 28 px *and* the primaries at their minimums. Hiding the strip is the first relief.
  - **Height budget at 800×600** (rail 64, page padding 16):
    - stage width = 800 − 64 − 32 = **704**;
    - header 44 + gap 12;
    - control bar: rows A and B 44 + 44, gap 8, padding 2×12, total 120;
    - stage height = 600 − 32 − 56 − 120 − 12 = **380**. The 12 px gap between stage and control bar applies at every size;
    - one notice (≤ 52 px) leaves **328**.
    - Wide mode at 704×380: the gauge column is about 300 px wide (power about 64 px). The right column holds 2×2 primaries at **150 px** per row (2×150 + an 8 px gap = 308; values about 48 px), then an 8 px gap, then the 64 px secondary strip. Total 308 + 8 + 64 = **380**.
    - At 704×328 (notice shown), the secondary strip hides first, and the primaries get 2×160.
  - **Other reference sizes:**
    - **1280×800:** the sidebar shows (≥ 1100), so stage width = 1280 − 200 − 32 = **1048**. That is below 1100, so rows A and B **don't** merge (control bar 120). Stage height = 800 − 32 − 56 − 120 − 12 = **580**.
    - **1920×1080 windowed:** the sidebar is 200 px, so stage width = **1688**. Rows merge at ≥ 1100 stage width (control bar 68). Stage height = 1080 − 32 − 56 − 68 − 12 = **912**.
    - **1920×1080 fullscreen:** no nav, so stage width = 1920 − 32 = **1888**. Stage height is **912**.
  - **Priority:** controls (AC11) are never shrunk or hidden. Then the secondary strip hides. Then the numbers shrink, down to 80 % of the AC12 minimums, and only while a notice is shown.
  - **Observer cost:** the ResizeObserver callback is rAF-throttled and writes the CSS variables only when a rounded value changes.
  - The observer writes these as CSS variables, and components read them.
- **Why:** this is deterministic and unit-testable, so the AC12 thresholds can be tested on the pure function. It needs no modern CSS, and it is robust to the nav hiding, fullscreen, and the browser's own zoom.
- **Rejected:**
  - (a), because `vh` ignores the nav and control bar, so it either overflows or under-uses the space.
  - (c), because transforms blur text and break hit areas.

### ADR-4: Esc routing and fullscreen as one small key layer
- **Context:** the layout's global Esc toggles the nav, and native `<dialog>` closes on the same keypress (G8). Fullscreen today only hides the nav.
- **Decision:**
  - **Overlay state comes from the DOM**, not a counter: `overlayOpen = !!document.querySelector('dialog[open], [data-overlay="open"]')`, read at keydown time. *(Changed per design review: a counter can leak when a dialog unmounts while open.)* Future popovers set `data-overlay="open"`.
  - **`src/lib/ui/keys.ts`** exports a pure function `routeKey(e: {key, ctrlKey, metaKey}, s: {overlayOpen, fullscreen, typing})`. It returns `'none' | 'exit-fullscreen' | 'toggle-fullscreen'`:
    - **Esc:** `overlayOpen` → `'none'`, so the native dialog handles it; else `fullscreen` → `'exit-fullscreen'`; else `'none'`.
    - **F11:** returns `'toggle-fullscreen'` unless `typing`.
    - `typing` means the event target is an `input` (other than checkbox, radio or button types), a `textarea`, a `select`, or `[contenteditable]`.
    - **Ctrl+Cmd+F is deliberately not handled in JS.** On macOS, Tauri's default app menu already binds it (View › Enter Full Screen) and toggles native fullscreen. Handling it in JS too would toggle twice. The store picks up the change through `onResized`.
    - The tooltip shows "F11" on Linux and "⌃⌘F" on macOS.
  - **`src/lib/stores/fullscreen.ts`:**
    - `fullscreen` is a readable store kept in sync from `getCurrentWindow().isFullscreen()`. It checks on mount, and on `onResized` debounced by 150 ms, with a second check 700 ms later because macOS can report a stale value mid-animation. It fires when the OS enters or exits fullscreen (green button, menu, gesture, GNOME shortcut).
    - `setFullscreen(on)` calls `getCurrentWindow().setFullscreen(on)`.
    - Capability additions: `core:window:allow-set-fullscreen`, `core:window:allow-is-fullscreen`.
    - Outside Tauri (the harness), it falls back to the DOM Fullscreen API and then to state-only, so the harness can exercise it.
  - **`+layout.svelte`:**
    - hides the nav when `$fullscreen`;
    - shows the "Press Esc to exit fullscreen" hint for 3 s on entering;
    - renders the floating "Exit fullscreen" button (`.btn .btn-secondary`, 44 px, top-right). It stays in the DOM and tab order, hidden only with `opacity: 0` (never `display: none` or `visibility: hidden`). It becomes `opacity: 1` for 2.5 s after any `pointermove`, and whenever it is `:focus-visible`. While hidden it has `pointer-events: none`. The harness skips elements with `opacity < 0.05`, as it already does.
    - The `toggle-fullscreen` window event and `navCollapsed` are removed.

### ADR-5: One chart theme module
- **Decision:** `src/lib/charts/theme.ts` exports:
  - `CHART_TEXT`: axis 12, legend 13, label 12, tooltip 13, plus colours equal to the CSS tokens;
  - `SERIES` with one hex per series:
    - power `#ff4d6d`;
    - hr `#ff6b60` (red family, already used on history detail);
    - cadence `#64b5f6`;
    - speed `#4caf50`;
    - cmdPower `#ffb74d` (dashed);
    - ftp `#ffb74d`;
    - ctl `#4a90d9`;
    - atl `#ff4d6d`;
    - tsb `#9a9ab2`.
  - `POWER_ZONE_COLORS[7]` and `HR_ZONE_COLORS[5]`. These replace the five palettes in settings, ZoneRideStatus, ZoneDistribution, PowerHistogram and HrHistogram, using the ZoneDistribution values as the canonical set. PowerGauge bands derive from `POWER_ZONE_COLORS`.
  - Helpers `axisDefaults()`, `legendDefaults()`, `tooltipDefaults()` and `gridDefaults()`, each returning an object to spread into an option.
- **Test:** `theme.test.ts` asserts every `fontSize` is ≥ 12, and that each text colour has contrast ≥ 4.5 against `CHART_BG` (`--bg-surface`).

### ADR-6: Page width containers
- **Decision:** two global classes:
  - `.page`: `max-width: 1280px; margin-inline: auto; width: 100%`;
  - `.page-wide`: `max-width: 1600px`.
- **Where each is used:**

  | Page | Container |
  |---|---|
  | Ride | Full width (no container) |
  | Devices, History, History detail, Analytics | `.page-wide` |
  | Settings, About | `.page`, with two columns at ≥ 1100 px content width |

- **Why this meets AC6:** at 1920×1080 the sidebar is 200 px and the content area is about 1672 px after padding. So `.page` leaves about 23 % empty and `.page-wide` about 4 %, both under the 30 % limit.
- **Scroll container:** `main.content` stays the scroll container for every page except Ride (G7, deferred; scroll restoration is out of scope).
  - On `/`, the layout adds `class:fit` to `main.content`, which sets `overflow: hidden; padding: 16px; display: flex; flex-direction: column`.
  - The ride page root is `flex: 1; min-height: 0`.
  - So the ride page is exactly the viewport height and can't scroll, which AC11 needs.
- **Breakpoints:** all non-ride breakpoints are media queries on **window width**. Where this document or the spec gives a content width, the window-width equivalent is: content ≥ 1024 → window ≥ 1280; content < 960 → window < 1280; "≥ 1100 content" for two columns → window ≥ 1280. The nav width is known for each window width: a 64 px rail below 1100 and a 200 px sidebar at 1100 and above. The ride screen alone uses its measured stage size.

### ADR-7: Navigation
- **Wide windows:** at a window width of 1100 px or more, the nav is a 200 px sidebar showing icon and label.
- **Narrow windows:** below 1100 px, it is a 64 px rail with 44×44 items. The label shows as a tooltip on `:hover` and on `:focus-visible`, as a sibling `<span class="rail-tip" role="tooltip">` rather than `::after`, so it also works with the keyboard.
- **Icon changes:** the Ride icon becomes a play-in-circle glyph, so it no longer looks like the Analytics line glyph. The Devices icon becomes a simpler radio/sensor glyph.
- **Accessible names:** rail links keep `aria-label={label}`. The tooltip span is `aria-hidden`, because the accessible name already carries the label.
- **Device status:** the bottom block becomes a `DeviceStatus` "nav" variant:
  - sidebar: "3 connected", with a coloured dot and short name for each live channel;
  - rail: "3", with a tooltip listing the devices.

### ADR-8: The test harness is committed as a dev tool
- **Location:** move `ui-harness` into `tools/ui-harness/`, with its own `package.json` pinning `playwright@1.57.0`. It is not part of the app's dependencies or build. `webkit-libs/` (Ubuntu `.so` files, for Fedora only) is not committed; the README explains how to get WebKit on Fedora, and WebKit is skipped if it is missing.
- **Checks:** add a check mode, `node shoot.mjs --check`, that computes the spec's success-signal counts from the per-page metrics and exits non-zero on any failure:
  - text below 12 px;
  - targets below 32 px;
  - contrast failures;
  - horizontal overflow;
  - ride controls out of viewport;
  - numbers clipped;
  - the AC6 centring and empty-width rules at 1920.

  The checks cover the states `normal`, `empty`, `error`, `riding` and `riding-zone`. There are documented, minimal exemptions: inline paragraph links, and the dev toolbar, which is a dev-only build.

  Additional counters and scripted checks, one per AC that the harness proves *(added per design review)*:

  | AC | Check |
  |---|---|
  | AC1 | All non-localhost requests are routed to a never-resolving handler. Assert that **zero** were attempted, and that the nav is visible within 1 s of navigation. |
  | AC2 | No interactive element computes a `font-family` starting with Arial, Times, Liberation, DejaVu Serif or `serif`. Each control's family must equal its role's token: the sans stack for buttons and inputs, and the data stack for `.data` inputs. |
  | AC6 | Centring and empty-width at 1920. On review pages, every `[data-metric-card]` is ≤ 360 px wide. |
  | AC8 | At 1280 and 1920, every nav link has visible label text. At 800, focusing each rail link by keyboard (Tab) makes its `role=tooltip` visible. The device indicator has an accessible name matching `/\d+ devices? connected\|None connected/`. |
  | AC9 | Open the Edit dialog and press Esc. The dialog closes and the nav stays visible. Enter fullscreen, open a dialog and press Esc: still fullscreen. Press Esc again: fullscreen exits. |
  | AC10 | `page.clock` is used: the hint is visible after entering fullscreen and gone at +3.1 s. The exit button is visible after a `pointermove` and after Tab focus. |
  | AC11 | Ride controls lie inside the viewport in every view and state. |
  | AC12 | Number sizes and no clipping (bounding box inside container). Empty space below the last element is ≤ 15 % of viewport height at 1920 fullscreen. |
  | AC13 | In the `empty` state, the gauge value and every metric card read `--`. |
  | AC20 | At 800×600 the table has no horizontal overflow, the title column is ≥ 24ch, and the sort icon is ≥ 12 px. |
  | AC24 | `--flows` runs a scripted, deterministic flow walk against the mock: start, pause, resume and stop a ride; post-ride Done; zone ride start and stop; trainer mode and adjust; device connect, disconnect and details; open a history item; edit and save; export; analytics; settings save and discard. Every step asserts the mock received the expected command. |

  | AC5 (ride) | Every `[data-ride-control]` is at least 44 px tall. That includes the zone-builder dialog controls, which carry the hook (AC15). |
  | AC11 worst case | A scenario at 800×600 in `riding-zone`: emit `device_reconnecting` through `__mockEmit` so a notice shows, then assert every `[data-ride-control]` is inside the viewport and the values meet the 80 % minimums. |

  All timed behaviour uses Playwright `page.clock`, with no sleeps. In the harness, the fullscreen store uses its **state-only** fallback (no DOM Fullscreen API), so the browser's own Esc handling can't interfere with AC9. AC1's paint check reads `performance` paint entries with a 1 s budget measured in the page, not wall-clock time in the runner. The DOM `.gauge-value` is measured against `--bg-surface`, its container's background; canvas pixels aren't sampled.

  **Missing engines:** if WebKit isn't available, `--check` **fails**, unless `--engines=chromium` was passed explicitly, in which case it prints a warning.

  **CI:** `ui:check` is a local tool and doesn't run in CI, because browsers and WebKit libraries are heavy and Fedora needs workarounds. Every unit-testable rule (tokens, chart theme, key routing, `rideLayout`, dirty tracking, legend filter, segmented keys) is in vitest and runs in CI.
- **npm script:** `"ui:check": "node tools/ui-harness/shoot.mjs --check --no-shots"`.

## Tokens (app.css)
| Token | Old | New | Notes |
|---|---|---|---|
| `--text-primary` | #f0f0f5 | same | |
| `--text-secondary` | #a0a0b8 | same | 6.0–7.4:1 |
| `--text-muted` | #70708a | **#8c8ca6** | 4.68–5.81:1 on all surfaces |
| `--text-faint` | #44445a | **#6c6c88** | **Never body text.** Used only for disabled states and decoration, at ≥ 3.02:1 |
| `--danger-text` | (new) | #ff6b60 | ≥ 5.49:1; red text uses this instead of `--danger` |
| `--accent-strong` | (new) | #d42a47 | White text 4.97:1 (primary button fill) |
| `--success-strong` | (new) | #2e7d32 | White text 5.13:1 (Start fill) |
| `--danger-strong` | (new) | #c62828 | White text 5.62:1 (Stop fill) |
| `--text-xs` | 11 px | **12 px** | The floor |
| `--text-sm` | 12 px | **13 px** | |
| `--text-base` | 15 px | 15 px | |
| `--text-lg` | 16 px | 17 px | |
| `--control-h` | (new) | 36 px | Default button and input height |
| `--control-h-lg` | (new) | 44 px | Ride controls |
| `--nav-rail-width` | 56 px | 64 px | Plus a new `--nav-sidebar-width: 200px` |

- **Tinted group panels:** these on history detail must use the blended background in the contrast test. The test composites each panel's rgba over `--bg-body` and checks `--text-muted` against it.
- **Form-control reset** (fixes G1):
  - `button, input, select, textarea { font: inherit; color: inherit; }`;
  - `input, select, textarea` get `min-height: var(--control-h)`, the `--bg-input` background and the default border;
  - `color-scheme: dark` on `:root`.
- **Themed form controls (AC22):**
  - `select`: `appearance: none`, with an inline-SVG chevron as `background-image` at the right and `padding-right: 32px`.
  - Checkboxes (anywhere besides `Switch`): always inside a `<label class="check-row">` that is at least 32 px tall and is the click target. The harness measures that label instead of the bare input: for checkbox and radio inputs inside a label, the label's box is used. The `.check` class has `appearance: none`, an 18×18 box, the `--border-strong` border, an `--accent-strong` fill and an SVG tick when `:checked`, and the focus ring. The ride "Auto" checkbox becomes `Switch`.
  - `input[type=date]`: field chrome (height, background, border, radius, font) matches the other inputs. `::-webkit-calendar-picker-indicator` is restyled: `filter: invert(0.8)` and a 20 px hit area. The popup picker stays native, which the spec allows.
  - Number inputs hide spinners (`::-webkit-inner-spin-button { appearance: none }`), because WebKitGTK spinners are tiny.
  - Toggles: Male/Female and Metric/Imperial become `SegmentedControl`.
- **Tinted panel tokens:** the history-detail group panels become tokens in `app.css`: `--panel-time`, `--panel-effort`, `--panel-power`, `--panel-hr` and `--panel-move`, as rgba values. The token test composites them over `--bg-body`.
- **Token test:** `src/lib/ui/tokens.test.ts` reads `src/app.css` with `fs` and parses `--name: value` pairs. For every text token in `TEXT_TOKENS` against every surface in `SURFACE_TOKENS`, plus the tinted panels, it asserts ≥ 4.5. It asserts `--text-faint` ≥ 3, and asserts the strong fills' white text ≥ 4.5.

## Shared components (`src/lib/ui/`)
| Component | Props | Behaviour |
|---|---|---|
| `IconButton` | `label` (required), `icon` snippet, `size='md'\|'lg'`, `variant='ghost'\|'secondary'`, `onclick`, `disabled`, `tooltipSide='top'\|'right'\|'bottom'` | Renders a `.btn .btn-icon` button (36 or 44 px) with `aria-label={label}`. A `role=tooltip` span shows on hover and focus-visible after 300 ms. |
| `SegmentedControl<T>` | `options: {value: T, label: string}[]`, `value` (bindable), `label` (aria), `size='md'\|'lg'`, `onchange?` | `role=radiogroup`. Each button is `role=radio` with `aria-checked`, and roving `tabindex` gives the group one tab stop. Arrow, Home and End keys move the selection (logic in `segmented.ts`, tested). Heights are 32 and 44 px. |
| `Dialog` | `open` (bindable), `title`, `onclose`, `closeOnBackdrop=false`, `width='sm'\|'md'\|'lg'`, `children` and `footer` snippets | Wraps a native `<dialog>` with `showModal()`. The header has the title and a "Close" `IconButton`. The native `cancel` event (Esc) closes it. On close, focus returns to the element that was focused before opening. `showModal` is skipped if the dialog is already open, so it is idempotent. On destroy while open, it calls `close()`. The body scrolls inside `max-height: min(80vh, 720px)`. |
| `Notice` | `tone='info'\|'success'\|'warning'\|'error'`, `title?`, `children`, `actions?` snippet, `ondismiss?` | Icon, text and actions. When `ondismiss` is set, it shows a "Dismiss" `IconButton` (36 px). Uses `role=status`, or `alert` for errors. |
| `StateView` | `kind='loading'\|'empty'\|'error'`, `title`, `message?`, `details?`, `action?: {label, onclick}`, `icon?` | A centred block. Errors show the title, the message and `<details><summary>Details</summary>{details}</details>`, with a primary "Retry" button from `action`. The action button is disabled while its promise is pending, so it can't be triggered twice. |
| `Switch` | `checked` (bindable), `label`, `disabled` | A `<button role=switch aria-checked>` with a 44 px row target and a visible label. Replaces the native "Auto" checkbox. |

These are tested through the pure logic they use (key routing, segmented arrow navigation in `ui/segmented.ts`). Rendering is checked in the harness, which matches the repo's current practice of no component tests.

## Screen designs

### Ride (`/`)
- **Header row** (44 px):
  - `SegmentedControl` Gauges / Graphs / Retro (lg) on the left.
  - On the right, `IconButton` "Enter fullscreen (F11)", or "(⌃⌘F)" on macOS (detected via `navigator.userAgent` containing "Mac").
- **Stage** (`RideStage.svelte`, new). It observes its size and computes `rideLayout(w, h)`.
  - **Gauges view:**
    - `wide`: CSS grid `1fr 1.2fr`. The left cell is the gauge with the power value overlaid. The right cell is a 2×2 grid of primary MetricCards (HR, Cadence, Speed, Time). A secondary strip (3s Power, NP, TSS, IF) runs full-width under both when `showSecondary`.
    - `stacked`: gauge on top, a 4-across primary row, then the secondary strip.
  - **PowerGauge:**
    - Remove the ECharts `detail` and `title`. The value is a DOM element `.gauge-value` positioned at the gauge's centre-bottom, with `font-size: var(--power-px)`. It renders `--` when there is no reading (AC13).
    - Axis labels 12 px.
    - The band colours come from `POWER_ZONE_COLORS` via the FTP fractions.
  - **MetricCard:**
    - New prop `size='stage'`, which reads `--value-px` (primary) or `--small-value-px` (secondary).
    - Value and unit on one line: `white-space: nowrap`, unit at 0.45em.
    - Label 12 px uppercase `--text-muted`.
    - The accent glow stays.
    - The tooltip uses tokens.
  - **Graphs view:** MetricsChart fills the stage (height 100 %). Resizing calls `chart.resize()` and never re-initialises. `SERIES` colours.
    - Legend `data` comes from `visibleSeries(hasData, zoneActive)` in `utils/liveChart.ts`, tested. "Cmd Power" appears only while a zone ride is active.
    - The legend is recomputed only when that set changes, not on every sensor event.
  - **Retro view:** RetroRide sized `width: min(100%, (stage-h) * 16/9)`, centred.
- **Control bar** (`.ride-controls`, part of the page, never scrolled away):
  - **Row A:**
    - Start (`.btn-lg .btn-success`) or Stop (`.btn-lg .btn-danger`), min-width 140 px;
    - Pause/Resume (`.btn-lg .btn-secondary`);
    - `Switch` "Auto start/stop";
    - the zone-active badge when applicable.
  - **Row B** (when a trainer is connected):
    - `TrainerControl`: mode `SegmentedControl` (lg), adjust buttons `.btn-lg` (min 52 px wide), and the target value at 28 px data font;
    - the "Zone ride" toggle button (`.btn-lg`).
    - At a stage width of 1100 px or more, rows A and B merge into one row.
  - **During a zone ride**, row B shows the compact `ZoneRideStatus`:
    - zone badge (≥ 4.5:1);
    - target range;
    - current value;
    - time in zone;
    - a progress bar;
    - a "Stop zone ride" `.btn-lg .btn-secondary`.
    - All text ≥ 12 px, and the duplicate "Power Zone 3" heading is removed.
  - **Notices** (`lifecycleError`, zone result, trainer error) render as `Notice` above row A, inside the control bar, so they never push controls off-screen. Only the latest one shows; it has a "Dismiss" button, and the trainer error still auto-clears.
- **Config load failure on Ride:** a `Notice` warning reading "Couldn't load your settings, so zones use defaults (FTP 200 W)", with a "Retry" action. Riding still works.
- **Notice precedence** (in the control bar): one notice shows at a time, highest priority first:
  1. lifecycle error;
  2. trainer error;
  3. connection health;
  4. config warning;
  5. zone-ride result.

  Lower-priority notices wait and appear when the higher one is dismissed or clears. None are dropped.
- **ZoneRideBuilder** opens in a `Dialog` (md) titled "Start a zone ride". Its controls count as ride controls, so chips, inputs and buttons are **44 px**:
  - mode `SegmentedControl` (Power / Heart rate);
  - zone chips as a `SegmentedControl`-style radiogroup with `.btn-lg` items, each showing "Z3 · 176–210 W";
  - number inputs at 44 px;
  - footer: "Cancel" and "Start zone ride" (`.btn-primary`).
- **ConnectionHealth:** `Notice` (warning/error) shown in the control bar's notice slot. Its action button becomes `.btn`.
- **Post-ride ActivityModal:** see below.

### ActivityModal
- Becomes a `Dialog` (md) titled "Ride saved". In edit mode the title is "Edit ride".
- Title input 36 px. Activity-type chips are a radiogroup (`role=radio`, `aria-checked`) of 36 px `.btn` items. RPE uses the same pattern.
- **RPE:**
  - a 10-button row of 36 px;
  - with "Easy" under 1 and "Max" under 10, at 12 px;
  - the selected state is a filled `--accent-strong` with white text.
- **Footer:**
  - post-ride: "Done" (`.btn-secondary`) and "Save details" (`.btn-primary`). Helper text under the title reads "Your ride is saved. Add details now or later from History.";
  - edit: "Delete ride" (`.btn-ghost`, danger text, left), "Cancel" and "Save". Delete keeps its existing confirmation step, with no behaviour change.
- Esc and the close button behave like Done or Cancel, and the backdrop does nothing (Q3).

### Devices
- `.page-wide`. The header has "Devices" and a "Scan" `.btn-primary` (with a spinner while scanning, and "Stop" while searching).
- Sections are kept. Each device becomes a **full-width row**:
  - **Name**, 15 px semibold, never truncated before 32 characters, with the transport badge(s) beside it.
  - **Meta line (13 px, muted):** a status dot with its label ("Connected" or "Not in range"), a battery icon with %, a signal icon with its label, and "Primary" as a badge.
  - **Actions (right-aligned `.btn`):**
    - "Details" is a labelled button, not an icon.
    - "Set as primary" appears when applicable, and "Unlink".
    - Disconnect (`.btn-secondary` with danger text) and Connect (`.btn-primary`).
  - **Linked devices:** a single "Disconnect" plus an "Also connect ANT+" secondary, so the actions don't read as two devices.
- At 800 px, actions wrap below the meta line.
- **Out-of-range rows:** opacity is removed. Text uses `--text-secondary`, and the status is a grey dot with "Not in range".
- **Details** opens in a `Dialog` (lg). GATT UUIDs use `--text-muted` with data font 12 px. Characteristic chips are 12 px.
- **SetupBanner** becomes a `Notice tone=warning` with:
  - title "Finish Bluetooth/ANT+ setup";
  - the checklist at 13 px;
  - text "Fix setup will ask for your administrator password once.";
  - actions "Fix setup" (`.btn-primary`) and "Dismiss" (hidden until restart, via an in-memory store).
- Empty: `StateView` "No devices yet" with the action "Scan for devices". Load error: `StateView` error with Retry.

### History list
- `.page-wide`. The header has "History", a count, and a `SegmentedControl` Cards / Table with visible labels.
- **Cards view:**
  - grid `repeat(auto-fill, minmax(340px, 1fr))`;
  - each card shows the title, date, type badge (12 px) and a 2×2 stats block (labels 12 px muted, values 18 px data);
  - "Export FIT" sits in a card action row as a `.btn` (36 px). The row is always visible (not hover-only), so keyboard and pointer users find it the same way.
  - **Stretched-link pattern:** the card is an `<article>`. The title is the only `<a href>`, and its `::after` covers the card, so clicking anywhere opens the ride. "Export FIT" is a sibling `<button>` with `position: relative; z-index: 1` above that overlay. No interactive element is nested inside the link.
- **Table view:**
  - At under 960 px of content width, the Avg HR, Work and Type columns are hidden. The title column is `min-width: 24ch`.
  - Sort indicator: a 12 px chevron icon in the header button (which is 32 px tall).
- **States:** loading (`StateView` loading), empty ("No rides yet", "Start a ride from the Ride screen" with a "Go to Ride" action), error ("Couldn't load your rides", details, Retry).

### History detail
- `.page-wide`. The back link is a `.btn .btn-ghost` "← History" (36 px).
- **Header:** title, badges and date. Actions "Edit" and "Export FIT" as `.btn-secondary`.
- **Summary:**
  - Groups (Time, Effort, Power, Heart rate, Movement) are each a labelled section: heading 12 px uppercase `--text-secondary` on the tinted panel, which passes contrast after compositing.
  - Cards in a group are `flex: 1 1 150px; max-width: 360px`.
  - Groups flow in a grid `repeat(auto-fit, minmax(320px, 1fr))`.
- **Charts:**
  - every segmented control becomes a `SegmentedControl` (md, 32 px);
  - the time series is `height: clamp(320px, 45vh, 560px)`;
  - side-by-side panels at ≥ 1100 px (Power curve | Zone distribution, Power hist | HR hist), each `clamp(300px, 38vh, 460px)`.
- **ZoneDistribution:** every zone row is always rendered, including 0:00. The time and % values are rendered beside the bar, not only in a tooltip. Colours come from the theme.
- **PowerCurve:** log x-axis ticks at 1 s, 5 s, 15 s, 1 min, 5 min, 20 min and 1 h, via a custom `axisLabel.formatter` and an `interval` on a log axis. If the ECharts log axis can't place exact ticks, use a category-style set of `markLine`s at those durations, with labels. The implementer picks whichever renders all five required labels (AC21).
- **States:** loading; error with Retry; not-found ("Ride not found", "It may have been deleted", with the action "Back to History").

### Analytics
- `.page-wide`.
- **Summary tiles:** `flex: 1 1 180px; max-width: 360px` (AC6). Label 12 px `--text-muted`, and values coloured only when meaningful. "Ramp" uses `--text-secondary`, not faint. A "Week TSS" of 0 shows "0", with "No rides yet this week" as a 12 px subtext.
- **Charts:** PMC full width (`clamp(320px, 45vh, 520px)`), with the trend charts in a two-column grid at ≥ 1100 px.
- **States:** loading, empty and error with Retry.

### Settings
- `.page`, two columns at ≥ 1100 px: "Athlete profile" (left) and "Training zones" (right).
- **Controls:**
  - unit and sex selectors are `SegmentedControl` (md);
  - "Estimate from resting/max HR" is a `.btn-secondary`;
  - every number input is 36 px with its unit suffix visible.
- **Zones:** power zone rows show "Z1 · Active recovery · ≤ 138 W" in `--text-secondary`. HR zones get names too: Recovery, Endurance, Tempo, Threshold, Max.
- **Save bar:**
  - `dirty = !configEquals(saved, current)`, where `configEquals` lives in `src/lib/utils/dirty.ts` and is tested. It accounts for the display weight conversion.
  - When dirty, a sticky bottom bar (`position: sticky; bottom: 0` inside `main.content`) shows "Unsaved changes", "Discard" (`.btn-ghost`) and "Save" (`.btn-primary`).
  - After saving, a `Notice tone=success` "Settings saved" appears and auto-hides after 3 s.
  - The old bottom Save button is removed.
- The orphaned `<label for="sex">` is fixed. The `SegmentedControl` gets `label="Sex"` instead.
- **States:**
  - load failure: `StateView` error "Couldn't load your settings", with details and Retry. The form is hidden until a load succeeds;
  - save failure: `Notice` error "Couldn't save settings", with the reason. The bar stays dirty and the edits are kept;
  - Save is disabled while a save is in flight.

### About
- `.page`, two columns at ≥ 1100 px: app info and version on the left; links and credits on the right.
- Links are `<a href={url}>` with an onclick that calls `openUrl` and `preventDefault`. Each one is a 36 px row with an external-link icon.

## Failure modes
| Failure | Handling |
|---|---|
| `setFullscreen` is rejected (missing capability, or a WM that refuses) | Catch it and show a `Notice` error: "Couldn't enter fullscreen. Use your system's fullscreen shortcut instead." The nav stays visible, and the state is re-synced from `isFullscreen()`. |
| The OS exits fullscreen by itself (macOS green button, a GNOME shortcut) | `onResized` → `isFullscreen()` resyncs the store, and the nav comes back. |
| ResizeObserver reports 0×0 (hidden tab, first paint) | `rideLayout` returns the minimum sizes. Never NaN. |
| A font file fails to load | The `font-data` stack falls back to `ui-monospace`. No network is involved. |
| WebKit lacks a CSS feature we rely on | Covered by the harness running WebKit. Container queries and `:has()` for layout are avoided entirely. |

## Error and empty copy
| Screen | Error title | Empty title and next step |
|---|---|---|
| Devices | Couldn't load your devices | "No devices yet". Turn on your sensors and press **Scan for devices**. |
| History | Couldn't load your rides | "No rides yet". Start one from **Ride**. |
| History detail | Couldn't load this ride | "Ride not found". It may have been deleted. **Back to History**. |
| Analytics | Couldn't load your training data | "Nothing to analyse yet". Analytics appear after your first ride. |
| Settings | Couldn't load your settings | n/a |

Every error state has a "Retry" button and "Details" holding the backend message.

## One-way doors
There are none in the strict sense: no schema, data or public API changes. These are the decisions that are costly to reverse, flagged for visibility:
- **Typographic identity:** dropping Outfit for the system font (user decision Q1).
- **Removed `toggle-fullscreen` window event:** it was internal only. A grep of `src` and `src-tauri` shows no other emitters or listeners.
- **New capability permissions:** `allow-set-fullscreen` and `allow-is-fullscreen`. They are additive and cheap to revoke.

## Rollout and rollback
- This ships in a normal release with no data or config changes.
- `tauri.conf.json`: window `1280×800`, `minWidth: 800`, `minHeight: 600`. Capability additions are additive.
- To roll back, revert the PR(s). Nothing persists that depends on the new UI. The localStorage key `historyView` is kept with the same values.

## Implementation plan
Delivered as **three stacked PRs** so each one stays reviewable. Each slice ends with `npm test`, `npm run check`, `npm run build` and a harness run on the touched routes in both engines.

**PR 1: Foundations and shell**
1. **S0, harness into the repo:** move it to `tools/ui-harness/` (no `webkit-libs`, no shots), and add `--check` mode with the success-signal counters and exemptions (ADR-8). Record the baseline failure counts from the current UI in `docs/design/ui-polish/baseline.md`.
2. **S1, foundations:**
   - bundle `@fontsource/ibm-plex-mono` and the system UI stack; remove Google Fonts from `app.html` and the CSP;
   - token changes and type floor;
   - form-control reset;
   - `controls.css` (buttons, inputs, `.page`/`.page-wide`);
   - `tokens.test.ts`;
   - window size and minimum in `tauri.conf.json`.
   - ACs: AC1, AC2, part of AC4, AC7.
3. **S2, shared components:** `IconButton`, `SegmentedControl` (plus `segmented.ts` with tests), `Dialog`, `Notice`, `StateView`, `Switch`; `charts/theme.ts` with `theme.test.ts`.
4. **S3, shell:**
   - `keys.ts` (`routeKey`, with tests) and the `fullscreen` store;
   - capabilities;
   - layout nav (sidebar/rail with focus tooltips, device status variant);
   - Esc routing, the fullscreen hint and the exit button;
   - removal of `navCollapsed` and the `toggle-fullscreen` event.
   - ACs: AC8, AC9, AC10.

**PR 2: Ride screen** (the riskiest; it comes first after the shell)

5. **S4, ride layout:** `rideLayout.ts` with tests. The AC12 thresholds are tested at stage sizes 704×380 (secondary shown, primaries ≥ 40), 704×328 (notice shown: secondary hidden, ≥ 80 % minimums), 1048×580, 1688×912 and 1888×912, along with the mode switch, the `showSecondary` relief order, the 28 px secondary floor, and 0×0 input. The harness then measures the real values; `RideStage.svelte`; control bar; PowerGauge DOM value; MetricCard `stage` size; idle placeholders. ACs: AC11, AC12, AC13.
6. **S5, ride controls:** TrainerControl on `SegmentedControl` and `.btn-lg`; `Switch`; notices in the control bar; ZoneRideBuilder in a `Dialog`; compact ZoneRideStatus; MetricsChart fill, dynamic legend and `SERIES`; RetroRide sizing. ACs: AC14, AC15, part of AC17.
7. **S6, ActivityModal** on `Dialog`. Part of AC17.

**PR 3: Review screens**

8. **S7, Devices and SetupBanner:** rows, labelled actions, details `Dialog`, `StateView`. ACs: AC19, part of AC16, part of AC17.
9. **S8, History list and table:** cards grid, links, table column hiding, sort icon, states. ACs: AC20, part of AC16.
10. **S9, History detail and Analytics:**
    - groups;
    - `SegmentedControl`s;
    - chart heights;
    - all charts on `theme.ts` (SessionTimeseries, PowerCurve, PmcChart, TrendChart, PowerHistogram, HrHistogram, ZoneDistribution);
    - zones list and power-curve ticks;
    - states.
    - ACs: AC3 (canvas), AC21, part of AC16.
11. **S10, Settings and About:** two columns, save bar with `dirty.ts` and tests, zone names, links. ACs: AC18, part of AC5.
12. **S11, sweep:**
   - **docs:**
     - README gets a short "Using the app" note: F11 or ⌃⌘F for fullscreen, Esc to exit, the window minimum, and offline fonts;
     - add a "UI" entry to the release notes draft;
     - CONTRIBUTING gets `npm run ui:check` and the shared UI building blocks (`src/lib/ui/`, `controls.css`, `charts/theme.ts`).
   - **AC23 check:** `rg -n --glob '*.svelte' -e '^\s*(button|\.btn[\w-]*|\.[\w-]*-btn|\.[\w-]*button[\w-]*)\s*[,{:]' src/routes src/lib/components` must return nothing. The allowed exceptions are listed in this section with a reason, and none are expected.
    - AC23 grep: remove leftover per-file button styles and duplicated empty-state/spinner CSS;
    - the full `--check` in both engines;
    - AC24 flow walk in the harness;
    - update the baseline doc with the after numbers.

## Verification approach
| AC | Proof |
|---|---|
| AC1–AC6, AC8–AC13, AC20, AC24 | `tools/ui-harness --check` counters and scenarios, both engines |
| AC4 tokens, AC3 canvas, AC9 routing logic, AC12 thresholds, AC14 legend filter, AC18 dirty logic, segmented keys | vitest |
| AC14–AC17, AC19, AC21, AC22 | Harness screenshots, inspected, and listed in the Verification section of the handoff |
| AC7, real fullscreen | Manual check in `npm run tauri dev` on GNOME, and macOS on the user's machine (spec Q5) |
| AC23 | grep recorded in the handoff |
| AC24 | The suite plus a harness flow walk |
