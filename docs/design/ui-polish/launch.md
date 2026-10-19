# UI polish: launch

**Spec:** [spec.md](spec.md)   **Design:** [design.md](design.md)   **Verification:** [verification.md](verification.md)

## What ships
The frontend is redesigned on one set of shared building blocks:
- **Fonts and tokens:** the system UI font with bundled IBM Plex Mono, and contrast-safe tokens with a 12 px type floor.
- **Shared controls:** 32/44 px controls.
- **Navigation:** labelled, with one Esc rule and real OS fullscreen.
- **Ride screen:** fits any window and scales its numbers.
- **Review screens:** loading, error with Retry, and empty states throughout.

There are no backend behaviour, data or schema changes. Only these non-frontend changes are included:
- **`tauri.conf.json`:**
  - window 1280×800, minimum 800×600;
  - the CSP drops the Google Fonts sources.
- **`build:release` override:** gets the same window size and minimum. Before this, it replaced the windows array, so release builds had no size settings at all.
- **Capability:** `core:window:allow-set-fullscreen` and `allow-is-fullscreen`.
- **New dependencies:**
  - `@fontsource/ibm-plex-mono` 5.3.0 (OFL, bundled into the app);
  - `@types/node` 22.20.5 (dev only, for the token test).
- **New tooling:** `tools/ui-harness/` (dev only, not shipped) and `npm run ui:check`.

## Packaging: three stacked PRs
| PR | Branch | Commits | Contents |
|---|---|---|---|
| 1 | `feat/ui-polish-1-foundations` | `e21f984`..`013735b` | Harness and baseline, spec and design; fonts and tokens; shared components and chart theme; navigation, Esc and fullscreen |
| 2 | `feat/ui-polish-2-ride` | `1748f1a`..`747caef` | Post-ride and edit dialog; ride screen |
| 3 | `feat/ui-polish-3-screens` | `1ef200f`..branch head | Devices, History and detail, Analytics, Settings and About; docs; verification |

- PR 1 targets `main`, and each later PR targets the one before it.
- After each squash merge, rebase the next PR onto `main` and retarget it, as was done for #191–#195.
- Each PR builds and passes `npm test`, `npm run check` and `cargo test` on its own.

## Rollout
- This goes out in the next beta, after the real-device checks below.
- There is no feature flag. The change is purely visual and covers every screen, so a flag would mean shipping two UIs.
- **Release notes:** add the "Interface" section below to the next release.

## Before tagging: real-device checklist (the user)
1. **GNOME, `npm run tauri dev`:**
   - the window opens at 1280×800 and can't be dragged below 800×600;
   - F11 enters real fullscreen, Esc leaves it, the exit button appears when the mouse moves, and the nav returns.
2. **GNOME, ride:** a short ride with the simulator or real sensors. In a small window, Stop, Pause and the trainer buttons stay visible, and the numbers are readable from the bike.
3. **macOS, release build:** ⌃⌘F and the green button enter and leave fullscreen, and the nav comes back. Fonts render as SF Pro for the interface and Plex Mono for numbers.
4. **Both platforms:**
   - open the Edit dialog and press Esc: the dialog closes and the nav stays;
   - Settings: change something and the save bar appears; Save shows "Settings saved".
5. **Optional:** make the database read-only, then save Settings. An error notice should appear and your edits should be kept. This is AC18's failure path.

## Rollback
- Revert the merged PRs, most recent first. There are no data, schema or config migrations, and nothing persisted depends on the new UI. `localStorage['historyView']` keeps the same values.
- If only fullscreen misbehaves on one OS, the narrow fix is to stop calling `setFullscreen`: hide the button and the F11 handler. The rest of the UI stays.

## Observability
- The app has no telemetry, by design. The signals are:
  - `npm run ui:check`, which must stay at 0 failures in both engines;
  - the user's real-device pass;
  - bug reports after the beta.
- **Fullscreen failures** show as an on-screen notice ("Couldn't enter fullscreen…").
- **Failed loads** show as error states whose Details carry the backend message. Users can quote that message in bug reports.

## Release notes: "Interface" section (draft)
- **Readable everywhere.** Text is larger and clearer, with nothing smaller than 12 px, and every label and button meets accessibility contrast.
- **The ride screen fits your window.** Numbers grow as the window grows. Start, Stop, Pause and the trainer controls always stay visible, even in a small window.
- **Real fullscreen.** Press F11 on Linux or ⌃⌘F on macOS, or use the fullscreen button. Press Esc to leave.
- **Esc does one thing.** It closes the open dialog, otherwise it leaves fullscreen. It no longer hides the navigation.
- **Labelled navigation.** Labels show in wider windows, and tooltips (on hover and keyboard focus) in narrow ones.
- **Clear errors.** When something can't load, you see what happened and a Retry button, instead of a raw database message.
- **Devices:** full device names, labelled buttons, and battery and signal at a glance.
- **History and ride detail:** a card grid, a table that fits small windows, every zone with its time, and the power curve labelled at 5 s, 1, 5 and 20 min, and 1 h.
- **Settings:** a save bar appears when you have unsaved changes.
- **Works offline.** The interface uses your system font and makes no network requests. It used to wait on Google Fonts and could stay blank for half a minute on a bad connection.
