# UI polish: verification

**Branch:** `feat/ui-polish`   **Date:** 2026-10-19   **Spec:** [spec.md](spec.md)

## Runs this record is based on
- **`npm test`:** 282 tests in 23 files, all passing. The new test files are:
  - `ui/tokens.test.ts`
  - `ui/keys.test.ts`
  - `ui/segmented.test.ts`
  - `charts/theme.test.ts`
  - `utils/rideLayout.test.ts`
  - `utils/dirty.test.ts`
  - `utils/liveChart.test.ts` (adds `visibleSeries`)
- **`npm run check`:** 0 errors, 0 warnings.
- **`npm run build`:** succeeds.
- **`cargo test`:** see "Rust" below.
- **`node tools/ui-harness/shoot.mjs <out> --check`:** a full run on both engines, Chromium 143 and WebKit 26 (WPE).
  - It covered 7 routes × their states (`normal`, `empty`, `error`, `riding`, `riding-zone`, `prereq=fail`) × 800×600 / 1280×800 / 1920×1080, plus the interactions and every scripted scenario.
  - Result: **CHECK chromium PASS (0 failures)** and **CHECK webkit PASS (0 failures)**, with 204 screenshots per engine and 0 harness errors.
  - The baseline on `main` was 4,620 failures in Chromium and 3,411 in WebKit ([baseline.md](baseline.md)).
- **Console:** the only error is `Session rehydration failed: … database is locked`. It appears on the 15 pages of the mock's `error` state, which is expected and unchanged from the baseline. There are no errors in any other state.

## Verify-gate review (2026-10-19) and fixes
An independent review found three blocking issues and several gaps in the harness. Everything below was re-run after the fixes.
1. **Arrow keys changed the trainer.** ↑/↓ on the new segmented controls and zone chips also moved the trainer target, through TrainerControl's window-level shortcut.
   - **Fix:** the shortcut now ignores keys that are already handled (`defaultPrevented`), typing targets (`isTypingTarget`, which now covers textareas too) and open dialogs.
   - **Test:** a regression scenario, "Arrow keys on a segmented control do not change the trainer".
2. **Analytics scrolled sideways in WebKit.** The grid used `1fr`, not `minmax(0, 1fr)`; History detail's `.two-col` had the same pattern. The harness missed it because it only checked document scroll, and pages scroll inside `main.content`.
   - **Fix:** both grids use `minmax(0, 1fr)`.
   - **Harness:** now fails on horizontal overflow in any scroll container.
   - **Negative test:** reverting the Analytics grid makes `--check` fail with `main.content 2076 > 1074`.
3. **The Settings error state was never run.** `['settings', 'error']` is now in the matrix.

**Other fixes:**
- **Harness:** clipped text now counts as a failure. That surfaced zone names cut off on ride detail, now fixed with a 13.5 rem name column. Unexpected console errors and pages the harness couldn't run now fail `--check`.
- **New scenarios:**
  - AC1 on a hanging network: FCP ≤ 1 s and zero external requests;
  - AC20: the table fits at 800×600, the title column is ≥ 24 ch, and the sort icon is ≥ 12 px;
  - AC17: Esc closes the zone builder and the post-ride dialog.
- **App:**
  - **Devices:** a successful Scan clears the load error.
  - **Ride detail:** "Ride not found" now matches the backend's "no rows returned".
  - **Zone ride:** Start has an in-flight guard and shows its error inside the builder.

**After the fixes:** `--check --no-shots` passes in Chromium and WebKit with 0 failures each, and `--check --only=-normal` with screenshots passes in both too.

**Second verify pass:** it confirmed all three blockers are resolved and regression-tested. It saw one Chromium flake in the devices flow: Scan was clicked while the button was still settling.
- **Harness fix:** flow steps now poll for the expected backend commands (up to 3 s) instead of sleeping, and the scan step waits for a settled, enabled "Scan for devices".
- **Minor fixes from that pass:**
  - "Ride not found" matches only the missing-ride errors;
  - a scan after a load error reloads the page state, including primaries;
  - a zone-start error hides once the target changes;
  - the trainer shortcut ignores every `<input>`, as it did before.
- **Stability:** three consecutive full `--check` runs pass in both engines, 6 of 6 green.

## Acceptance criteria
| AC | Status | How it's proven |
|---|---|---|
| AC1 Bundled fonts, no requests, fast paint | ✅ | **Harness:** zero non-localhost requests on every page in both engines. **Scenario** "AC1 hanging network": FCP ≤ 1 s, nav visible, and zero external requests attempted. **Fonts run:** first UI paint is 0.20–0.75 s, against about 28 s before. Google is removed from `app.html` and the CSP. |
| AC2 Controls use app fonts | ✅ | **Harness:** zero controls in a fallback font. The baseline had 1,053 in Chromium. |
| AC3 Text ≥ 12 px (DOM and canvas) | ✅ | **DOM:** zero text nodes under 12 px in either engine. **Canvas:** `theme.test.ts` asserts every chart `fontSize` is ≥ 12, and a grep of the chart components for `fontSize` under 12 finds none. |
| AC4 Contrast | ✅ | `tokens.test.ts` checks every text token × every surface and tinted panel at ≥ 4.5, faint at ≥ 3, and white on the strong fills at ≥ 4.5. `theme.test.ts` checks chart text. The harness found zero low-contrast nodes, against ≥ 2,061 before. |
| AC5 Targets ≥ 32 px, ride controls ≥ 44 px | ✅ | **Harness:** zero targets under 32 px, against 1,110 before, and zero `[data-ride-control]` under 44 px. That includes the zone-builder dialog controls. |
| AC6 Layout | ✅ | **Harness:** no horizontal scroll in the document **or any scroll container**, and no clipped text, at any size. At 1920 every page is centred or full-width, with ≤ 30 % of the content area empty. Every review-page metric card is ≤ 360 px. |
| AC7 Window 1280×800, minimum 800×600 | ✅ config / ⏳ live | It is set in `tauri.conf.json`, and also in the `build:release` override. That override previously replaced the windows array, dropping any size. **A live check in `npm run tauri dev` is still pending** (see "Not yet proven"). |
| AC8 Navigation labels | ✅ | **Scenarios:** "rail tooltips on keyboard focus" (Tab to each link at 800 px; the tooltip is visible), and "sidebar shows labels; device indicator is named". Both pass in both engines. |
| AC9 Esc does one thing | ✅ | **Unit:** `keys.test.ts` covers `routeKey`, `isTypingTarget` and `overlayOpen`. **Scenarios:** "Esc closes the dialog and leaves the nav alone", and "Esc in a dialog during fullscreen only closes the dialog" (a second Esc then leaves fullscreen). Both pass in both engines. |
| AC10 Real fullscreen | ✅ harness / ⏳ OS | **Scenario** "fullscreen hint and exit button", using `page.clock`: the hint shows and is gone at 3 s; the exit button shows on pointer move and hides 2.5 s later; it is reachable with Tab, visible on focus, and leaves fullscreen. **OS fullscreen** on GNOME and macOS is still pending. |
| AC11 Ride controls always visible | ✅ | **Harness:** zero ride controls off-screen across ride states, views and sizes. **Scenario** "worst case: 800×600 zone ride with a notice" passes in both engines. |
| AC12 Numbers scale, never clipped | ✅ | **Unit:** `rideLayout.test.ts` covers the minimums at the design's stage sizes, growth with size, strip relief and a 0×0 stage. **Harness:** no clipped or wrapped values, the minimums are met at 800×600 and 1920×1080, and empty space below the content at 1920 fullscreen is ≤ 15 %. |
| AC13 Idle placeholder `--` | ✅ | **Scenario** "idle values all read --" passes in both engines. |
| AC14 Graphs view | ✅ | **Unit:** `visibleSeries` (Cmd Power only during a zone ride). **Screenshots:** `ride-riding-graphs-*` show the chart filling the stage and the legend listing only series with data. One series palette lives in `charts/theme.ts`. |
| AC15 Zone ride panel | ✅ | **Screenshots:** `ride-normal-zonebuilder-*` show the builder as a dialog, fully visible at every size. **Harness:** its controls are ≥ 44 px. `ride-riding-zone-*` shows a compact status bar with no text under 12 px. |
| AC16 Error states with Retry | ✅ | **Screenshots:** `*-error-*` for devices, history, history-detail, analytics and settings (settings was added to the matrix after the verify review) each show the plain title, Details and Retry, with no empty state alongside. The ride screen shows a warning notice with Retry. |
| AC17 Dismiss everything | ✅ | **Scenario** "Esc closes the zone builder and the post-ride dialog". Notices have a labelled Dismiss of at least 32 px. **Screenshots:** `devices-prereqfail-*` shows the setup banner's Dismiss and the administrator-password text. Every dialog has a labelled Close button and closes with Esc (AC9 scenarios). The post-ride dialog says "Ride saved" and offers "Done" and "Save details". |
| AC18 Settings save bar | ✅ (except save failure) | **Scenario** "settings save bar, discard and save" at 800×600: there is no bar while clean; the bar appears inside the viewport after a change; Discard hides it; Save calls `save_user_config` and shows "Settings saved", which hides after 3 s. `dirty.test.ts` has 10 cases. **The save-failure path is not exercised,** because the mock can't fail a save while letting the load succeed. |
| AC19 Devices | ✅ | **Screenshots:** `devices-normal-*` show full names, labelled actions, and battery and signal each with an icon and text. **Harness:** no clipped text. |
| AC20 History table | ✅ | **Scenario** "AC20 history table fits 800x600": no overflow, title column ≥ 24 ch, sort icon ≥ 12 px. Passes in both engines. |
| AC21 Ride detail zones and power-curve ticks | ✅ | **Screenshot** `history-detail-normal-1920x1080-full`: all 7 power and 5 HR zones are listed, including those at 0:00, with time and %. The power curve labels 1 s, 5 s, 1 min, 5 min, 20 min and 1 h. |
| AC22 Themed form controls | ✅ | **Screenshots** in both engines: segmented controls replace the toggles, `select` has a custom chevron, and the date and number inputs are styled. The only checkbox, "Auto", is now a `Switch`. |
| AC23 Shared buttons | ✅ | The grep is recorded in design.md (S11). It returns only the documented layout modifiers and exceptions. |
| AC24 No regressions | ✅ (frontend) | **Harness:** unexpected console errors now fail `--check`. Analytics has no period control (it didn't before either), so "analytics periods" in the spec's flow list doesn't apply. |
| AC24 (flows) | ✅ | **Suites:** vitest, check and build all pass. **Scripted flows**, passing in both engines, each asserting the backend command the mock receives:<ul><li>start, pause, resume and stop a ride, then Done;</li><li>trainer SIM and ERG with adjustments;</li><li>start a zone ride from the builder, and stop a running zone ride;</li><li>devices: scan, details, disconnect and connect;</li><li>history: open a ride, edit and save, export;</li><li>settings: save bar, Discard and Save.</li></ul>`cargo test` is covered below. |

## Rust
`cargo test`: **625 passed, 0 failed, 1 ignored**, the same as before this branch. The capability additions (`core:window:allow-set-fullscreen` and `allow-is-fullscreen`) are validated when `tauri-build` runs.

## Not yet proven (needs a real machine)
1. **AC7:** check the window size and minimum in `npm run tauri dev` on GNOME.
2. **AC10:** check real OS fullscreen on GNOME (F11) and on macOS (⌃⌘F through the app menu, and the green button), including that the nav returns when the OS leaves fullscreen.
3. **Spec Q5:** the user does a visual pass on real WebKitGTK and WKWebView. The harness uses WebKit WPE as a stand-in.
4. **AC18:** the save-failure path. Either extend the mock with a "save fails" switch, or check it manually by making the database read-only.
