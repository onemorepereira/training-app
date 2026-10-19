# UI polish: baseline

These are the `tools/ui-harness --check --no-shots` results on `main` at `76e08e7` (v0.8.0-beta), taken on 2026-10-19 before any UI change.
- **Run:** Chromium 143 and WebKit 26 (WPE).
- **Pages:** 114 route, state and viewport combinations each.

A page lists at most 40 contrast failures, so the contrast counts are lower bounds. The `[data-page]` and `[data-ride-control]` counts are the measurement hooks that the redesign adds, not UI defects.

| Rule | Chromium | WebKit |
|---|---:|---:|
| AC4 contrast | ≥ 2061 | ≥ 2061 |
| AC5 target < 32 px | 1110 | 954 |
| AC2 control font (Arial) | 1053 | 0 (WebKit falls back to the system font) |
| AC3 text < 12 px | 144 | 144 |
| AC1 external request (Google Fonts) | 114 | 114 |
| Hook `[data-page]` missing | 114 | 114 |
| Hook `[data-ride-control]` missing | 24 | 24 |
| **Total** | **4620** | **3411** |

The only console error appears in the `error` mock state: `Session rehydration failed: ... database is locked`. That is expected.

## After (`feat/ui-polish`, 2026-10-19)
| Rule | Chromium | WebKit |
|---|---:|---:|
| Every rule above | 0 | 0 |
| Rules added later (container overflow, clipped text, console errors, ride controls < 44 px) | 0 | 0 |
| Scripted scenarios (AC1, AC8–AC11, AC13, AC17, AC18, AC20, AC24 flows, the arrow-key regression) | all pass | all pass |
| **Total failures** | **0** | **0** |

First paint on a hanging network went from about 28 s to under 0.8 s.
