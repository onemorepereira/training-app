# UI polish — Spec

**Track:** Standard   **Product stage:** Growth (v0.8 beta, Homebrew + Linux packages)
**Status:** Reviewed   **Last updated:** 2026-10-19
**Evidence:** visual audit from a mocked-backend harness (Chromium 143 + WebKit 26, 800×600 / 1280×800 / 1920×1080, normal / empty / error / riding states); findings G1–G11, R1–R11, M1–M3, D1–D6, H1–H5, HD1–HD6, A1–A3, S1–S3, AB1 are summarized in [Appendix: audit findings](#appendix-audit-findings).

## Problem
On both GNOME (WebKitGTK) and macOS (WKWebView), riders can't reliably read, find or act on what the app shows. Several things cause this:
- **Text** is small (down to 6.6 px, and 10 px across charts) or low-contrast (1.3–4:1).
- **Controls** are under-sized (20–29 px trainer and chart controls) and mixed in font (Arial controls next to Outfit labels).
- **Layouts don't adapt to the window.** At 800×600, Stop is below the fold during a ride. At 1920×1080, pages leave 750–900 px empty or stretch single numbers across 450 px cards.
- **Leaving or recovering is unclear.** Esc both closes a dialog and hides the navigation. Errors show raw database text next to "No sessions yet", with no retry. Notices dismiss only through a 16 px "×".
- **The UI can freeze on a bad network.** Fonts load from Google, and a hanging connection blanks the UI for about 28 s.

## Users
- **Rider on the bike:** reads live numbers from about 1 m away, often sweaty and mid-effort, and needs to hit Pause, Stop or trainer controls without hunting. Uses the app windowed beside other apps, or fullscreen on a laptop, desktop or TV.
- **Rider at the desk:** reviews history, ride detail and analytics, edits ride metadata, and manages devices and settings.

## Goals
- Everything is legible and clearly identifiable on both platforms, at every supported window size.
- Every element gets enough room. Layouts use the space available, so they are neither crowded at 800×600 nor sparse at 1920×1080.
- Anything shown can be read, reviewed and dismissed (or recovered from) in an obvious way.
- One consistent visual language, using shared building blocks instead of per-page styling.

## Non-goals
- A light theme or following the OS appearance setting. The dark look with the pink accent stays.
- Platform-native chrome (macOS overlay title bar, libadwaita header bars), and Windows support.
- New features, pages, metrics or data. No backend behaviour changes beyond window and fullscreen permissions.
- Redesigning the Retro view. It must keep working and fit the new layout, but its canvas art is unchanged.
- Touch input, localization, a leave-with-unsaved-changes prompt.

## Definitions
- **Supported viewports:** any window from 800×600 up to 2560×1440. Measured at 800×600, 1280×800 and 1920×1080.
- **Harness:** the mocked-backend Playwright run (`ui-harness/shoot.mjs`), with states `normal`, `empty`, `error`, `riding` and `riding-zone`, in Chromium and WebKit. In design.md this is a committed dev tool.
- **Interactive element:** any `button`, `a[href]`, `input`, `select`, `textarea`, `[role=button|tab|switch|checkbox|radio]` that is visible and enabled.
- **Ride controls:** Start/Stop, Pause/Resume, the trainer mode and adjust controls, and the zone-ride controls.

## User stories
1. As a rider on the bike, I can read power, HR, cadence, speed and time from the saddle, in a window or fullscreen, and the numbers get bigger when I give them more room.
2. As a rider on the bike, I can always see and hit Pause/Stop and the trainer controls without scrolling, even in an 800×600 window.
3. As a rider, I can tell what each navigation item and control is without hovering over it.
4. As a rider, when something fails I see what happened in plain words and can retry it; when something is shown to me I can dismiss it obviously; and pressing Esc does exactly one predictable thing.
5. As a rider at the desk, history, ride detail and analytics use a large window well, and stay usable in a small one.

## Acceptance criteria
Each AC is checked in both Chromium and WebKit, at all three measured viewports and all harness states, unless it says otherwise.

**Foundations**
- [ ] **AC1 (G2): fonts are bundled.** The app makes no network request at startup. Interface text uses the OS system UI font stack (`system-ui`, which gives SF Pro on macOS and the GNOME default on Linux). Numerals and data use a bundled monospace font. If the network is blocked or hanging, first paint happens within 1 s of navigation (harness).
- [ ] **AC2 (G1): controls use the app's fonts.** Every `button`, `input`, `select` and `textarea` computes the same `font-family` as surrounding text of its role. Measured: no interactive element reports Arial or Times, nor any UA default font.
- [ ] **AC3 (G5, chart text): minimum text size is 12 px.**
  - **DOM text:** no visible text node renders below 12 px CSS, including badges and chart tooltips (which are DOM). Measured by the harness.
  - **Canvas charts:** ECharts draws axis, legend and label text into a canvas. That text takes its font size and colour from a single shared chart theme module. A unit test asserts every `fontSize` in the theme is ≥ 12 and every text colour meets AC4 against the chart background. A grep finds no `fontSize` below 12 in chart components.
- [ ] **AC4 (G3, G4, D3, HD2, S2, R6, M2): text contrast.**
  - Every text colour token meets ≥ 4.5:1 against every surface token it can appear on (body, surface, elevated, hover, and tinted group panels). This is enforced by a unit test that parses the tokens.
  - In the harness, every visible text node meets ≥ 4.5:1 against its composited background, or ≥ 3:1 if it is ≥ 24 px (or ≥ 18.66 px bold).
  - Disabled controls and out-of-range devices are exempt from 4.5:1 but stay ≥ 3:1.
  - Text on filled buttons, including Start, Save and Stop, meets ≥ 4.5:1.
- [ ] **AC5 (R5, HD3, H4, D2, S3, M1, R10, AB1): target sizes.**
  - Every interactive element is at least 32×32 px, with no exceptions for links styled as buttons.
  - Ride controls are at least 44 px tall.
  - The only exception is inline text links inside a paragraph.

**Layout and navigation**
- [ ] **AC6 (G6, H2, A1, HD4): layout.**
  - No page scrolls horizontally at any supported viewport. A table may scroll inside its own container only below 960 px of content width.
  - At 1920×1080, every page's content block is horizontally centred in the content area: left and right gaps differ by ≤ 16 px, or the content spans the full width.
  - At 1920×1080, no page leaves more than 30 % of the content area's width empty.
  - On review pages (History, History detail, Analytics), no metric card holding a single value is wider than 360 px. The ride screen is exempt, because its cards grow with their numbers (AC12).
- [ ] **AC7 (window): window size.** The app window opens at 1280×800 by default and can't be resized below 800×600.
- [ ] **AC8 (G9, G10): visible navigation labels.**
  - When the window is at least 1100 px wide, navigation items show a visible text label next to the icon.
  - Below that, the icon-only rail shows each item's name on hover **and on keyboard focus**.
  - The connected-devices indicator at the bottom of the nav has a visible text label and an accessible name such as "3 devices connected".
    - In the sidebar, the label reads "3 connected".
    - In the rail, it shows a sensor icon and the count, with a tooltip listing the devices. With no devices it reads "None connected" in the sidebar and "0" in the rail. *(Amended 2026-10-19 per design review.)*
- [ ] **AC9 (G8, R4): Esc does one thing.** Pressing Esc does exactly one of these, in priority order:
  1. close the topmost open dialog, popover or menu;
  2. exit fullscreen;
  3. nothing.
  Esc never hides or shows the navigation. This is covered by a unit test on the key-routing logic and by a harness check reproducing "Esc in Edit modal" (the nav stays visible).
- [ ] **AC10 (R3, R4): real fullscreen.**
  - Fullscreen mode puts the OS window into real fullscreen (Tauri `setFullscreen`) and hides the nav.
  - It is entered from a labelled button on the ride screen or a keyboard shortcut. The button's tooltip names the shortcut.
  - On entering, a hint "Press Esc to exit fullscreen" appears for 3 s.
  - While in fullscreen, a visible "Exit fullscreen" button appears whenever the pointer moves or the button gets keyboard focus.
  - Leaving fullscreen by any route (Esc, the button, or the OS) restores the nav and the window state.

**Ride screen**
- [ ] **AC11 (R1): ride controls stay visible.** While a session is riding or paused, at every supported viewport and in every dashboard view (Gauges, Graphs, Retro), Start/Stop, Pause/Resume and the trainer controls (when a trainer is connected) are fully inside the viewport without scrolling.
- [ ] **AC12 (R2, R3): numbers scale and are never clipped.**
  - The power value is never clipped: its text bounding box lies inside its container.
  - The power value is ≥ 48 px at 800×600 and ≥ 112 px at 1920×1080.
  - HR, cadence, speed and time values are ≥ 40 px at 800×600 and ≥ 88 px at 1920×1080.
  - Secondary values (3s power, NP, TSS, IF), when shown, are ≥ 28 px.
  - These minimums apply while no notice is displayed. While one notice is shown, they may drop to 80 % of the minimum. The controls (AC11) always take priority over number size. *(Amended 2026-10-19 per design review.)*
  - Value and unit stay on one line at every supported viewport.
  - At 1920×1080 fullscreen, no more than 15 % of the viewport height is empty below the last ride element.
- [ ] **AC13 (R7): consistent idle state.** When there are no readings, the gauge and every metric card show the same placeholder "--".
- [ ] **AC14 (R8): Graphs view.**
  - The live chart fills the vertical space left after the controls.
  - The legend lists only the series that have data, so "Cmd Power" appears only during a zone ride.
  - Each series (Power, HR, Cadence, Speed) has one colour across the live chart, history detail and analytics, defined in a single module.
- [ ] **AC15 (R9, R10): zone ride panel.**
  - The zone builder opens where it is fully visible without scrolling at 1280×800 (as a dialog or a panel in view).
  - Its controls meet AC5.
  - The zone status panel has no text under 12 px.

**Feedback, review and dismissal**
- [ ] **AC16 (H1, HD1, A3): error states with retry.** Every page-level load failure shows an error state. That covers Devices, History, History detail, Analytics and Settings. On the Ride screen, a failed settings load shows a warning notice with Retry instead of a full error state, because riding must still work. The error state contains:
  - a plain-language title ("Couldn't load your rides");
  - the backend message under a collapsed "Details" section;
  - a "Retry" button that re-runs the load.
  
  The empty state never appears at the same time as the error state. An empty state means a successful load with no data, and says what to do next.
- [ ] **AC17 (R11, D4, M1): everything can be dismissed.**
  - Every inline notice and banner that can be dismissed has a visible, labelled close button of at least 32×32 px.
  - The setup banner can be dismissed for the session and states that "Fix" will ask for an administrator password.
  - Every dialog has a visible close button, closes with Esc (AC9), and labels its actions by outcome. In the post-ride dialog, the button that skips adding details says "Done, ride saved" or something equivalent. It doesn't say "Skip" with no explanation.
- [ ] **AC18 (S1): Settings save bar.**
  - While Settings has unsaved changes, a save bar stays visible at the bottom of the viewport at every supported viewport. It contains "Unsaved changes", "Discard" and "Save".
  - With no changes, the bar is hidden.
  - Saving shows a confirmation that disappears on its own.
  - A failed save shows an error notice with the reason. The bar stays, and the edits are kept.

**Per-screen**
- [ ] **AC19 (D1, D6): Devices.**
  - At ≥ 1024 px content width, device names of up to 32 characters show in full, with no ellipsis.
  - Every action has a visible text label (Info gets a label or tooltip plus aria-label).
  - Battery and signal strength show an icon and their value or label.
- [ ] **AC20 (H3): History table.** At 800×600 the table fits without horizontal scroll: lower-priority columns hide, and the title column shows at least 24 characters. Sort direction is shown by an icon of at least 12 px.
- [ ] **AC21 (HD5, HD6): History detail.**
  - Zone distribution lists every zone, including those with zero time, and shows each zone's time beside its bar without hovering.
  - The Power Curve x-axis labels each of 5 s, 1 min, 5 min, 20 min and 1 h that is no longer than the ride. *(Amended 2026-10-19 per design review.)*
- [ ] **AC22 (G11): themed form controls.** Checkboxes, toggles, selects and date inputs are styled to the dark theme and look the same in Chromium and WebKit, apart from the native popup menus a select opens.

**Consistency and safety**
- [ ] **AC23: shared buttons.**
  - Buttons, segmented controls, icon buttons, notices, dialogs and error or empty states come from shared components or shared classes.
  - Route and feature components define no button styles of their own: a grep of `src/routes` and `src/lib/components` for `.btn`/`button` style rules outside the shared module finds none, except intentional documented ones.
- [ ] **AC24: no regressions.**
  - `npm test`, `npm run check`, `npm run build` and `cargo test` pass.
  - The harness reports no new console errors.
  - Every existing user flow still works in the harness: start, pause, resume and stop a ride; post-ride dialog; zone ride start and stop; trainer mode and adjust; device connect, disconnect and details; history list and detail; edit metadata; export FIT; analytics periods; settings save.

## Success signals
- **Harness metrics** (committed tool; the counts are 0 in both engines):
  - text below 12 px;
  - interactive elements below 32 px;
  - text nodes failing AC4;
  - horizontally overflowing pages;
  - ride controls outside the viewport.
- **User check:** the user rides once on GNOME and once on macOS, windowed and fullscreen, and finds nothing they have to squint at, hunt for or can't dismiss.

## Constraints
- Svelte 5 runes, SvelteKit adapter-static, ECharts 6, Tauri 2.
- The CSP stays as strict as today or stricter. It can drop the Google Fonts allowances once fonts are bundled.
- Fonts are bundled as npm packages (for example `@fontsource/*`), pinned, with licences allowing redistribution (OFL).
- Changes are frontend-only, except `tauri.conf.json` (window size and minimum) and the capability permission for `setFullscreen`.

## Assumptions and open questions
| # | Item | Owner | Default |
|---|---|---|---|
| Q1 | Dropping Outfit for the system UI font changes the app's typographic identity. | User | Accepted (user chose "system font per OS + bundled fonts", 2026-10-19). |
| Q2 | Fullscreen keyboard shortcut. | Design | F11 on Linux and Ctrl+Cmd+F on macOS, matching each OS; both open the same handler. |
| Q3 | Should the post-ride dialog close when you click its backdrop? | Design | No, because a stray click would lose typed notes. Esc and the close button still close it. |
| Q4 | Does the harness move from scratchpad into the repo? | Design | Yes, as `tools/ui-harness/` (dev-only, not shipped) so it can verify this work and future UI changes. |
| Q5 | Real-device check on macOS (WKWebView) and GNOME (WebKitGTK). WebKit-WPE in the harness is a proxy. | User | User does the final check on both before release. |

## Appendix: audit findings
These are short summaries of the harness audit on 2026-10-19. "→ ACn" marks which acceptance criterion covers each one.

- **G1:** controls render in Arial (→ AC2).
- **G2:** fonts load from Google, and the UI is blank for about 28 s when the network hangs (→ AC1).
- **G3:** `--text-faint` is 1.3–2.0:1 (→ AC4).
- **G4:** `--text-muted` is 3.2–4.0:1 (→ AC4).
- **G5:** text goes as small as 6.6 px (→ AC3).
- **G6:** there is no shared content width (→ AC6).
- **G7:** the page uses a nested scroll container (→ design).
- **G8:** Esc closes the dialog and also hides the nav (→ AC9).
- **G9:** the nav is icon-only, with hover-only tooltips (→ AC8).
- **G10:** unlabelled device dots in the nav (→ AC8).
- **G11:** unthemed native checkbox and select (→ AC22).
- **R1:** Stop is below the fold at 800×600 (→ AC11).
- **R2:** the gauge value is clipped (→ AC12).
- **R3:** the ride screen doesn't scale up (→ AC12).
- **R4:** fullscreen is hidden and hard to leave (→ AC10).
- **R5:** trainer controls are 20–29 px (→ AC5).
- **R6:** Start is 2.78:1 (→ AC4).
- **R7:** the gauge shows "0" while the cards show "--" (→ AC13).
- **R8:** the Graphs chart has a fixed height, a stale legend and inconsistent colours (→ AC14).
- **R9:** the zone panel has tiny text (→ AC15).
- **R10:** the zone builder opens below the fold (→ AC15).
- **R11:** notices dismiss through a 16 px "×" (→ AC17).
- **M1:** "Skip" vs "×" is unclear, and targets are small (→ AC17, AC5).
- **M2:** button contrast is low (→ AC4).
- **M3:** RPE has no scale anchors (→ design, low).
- **D1:** device names are truncated in half-width cards (→ AC19).
- **D2:** device actions are unlabelled and small (→ AC19, AC5).
- **D3:** out-of-range devices are 1.28:1 (→ AC4).
- **D4:** the setup banner has no dismiss and no admin-prompt notice (→ AC17).
- **D5:** the details dialog is long (→ design, low).
- **D6:** battery and RSSI are bare text (→ AC19).
- **H1:** the error and empty states appear together (→ AC16).
- **H2:** History wastes the right side at 1920 (→ AC6).
- **H3:** the table overflows at 800, and its sort arrow is 6.6 px (→ AC20).
- **H4:** the view toggle is icon-only (→ AC5, AC8 pattern).
- **H5:** ride titles are repetitive (→ non-goal).
- **HD1:** the detail page has no error state (→ AC16).
- **HD2:** group labels are 1.7:1 (→ AC4).
- **HD3:** chart controls are 20 px (→ AC5).
- **HD4:** detail stretches to 1800 px (→ AC6).
- **HD5:** zero-time zones are missing from the list (→ AC21).
- **HD6:** the power-curve axis is sparse (→ AC21).
- **A1:** Analytics is capped at 1100 px (→ AC6).
- **A2:** Analytics labels are faint (→ AC4).
- **A3:** Analytics has no error retry (→ AC16).
- **S1:** Save is at the bottom of the page (→ AC18).
- **S2:** zone names are faint (→ AC4).
- **S3:** Settings toggles are small (→ AC5).
- **AB1:** About links are 19 px tall (→ AC5).
