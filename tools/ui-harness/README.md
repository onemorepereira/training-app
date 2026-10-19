# UI harness

This is a development tool. It isn't shipped and it isn't part of the app build. It renders the SvelteKit frontend in real browsers (Chromium and WebKit) with a **mocked Tauri backend**, so every screen and state can be screenshotted and checked without the Rust side or real sensors.

Use it to:
- **check UI rules** with `--check`. The rules come from [`docs/design/ui-polish/spec.md`](../../docs/design/ui-polish/spec.md);
- **produce screenshots** of every route, state and viewport, to review a UI change.

## Setup

```bash
cd tools/ui-harness
npm ci                       # pins playwright 1.57.0
npx playwright install chromium webkit
```

**WebKit on Fedora and other non-Ubuntu distros:** Playwright's WebKit build links against Ubuntu's ICU 74 and libjpeg8. Extract these files from the Ubuntu 24.04 packages `libicu74` and `libjpeg-turbo8` into `tools/ui-harness/webkit-libs/lib/`, and the harness preloads them:
- `libicudata.so.74`
- `libicuuc.so.74`
- `libicui18n.so.74`
- `libjpeg.so.8`

`webkit-libs/` is git-ignored. Without it, WebKit may fail to launch. In that case `--check` fails, unless you pass `--engines=chromium`.

WebKit matters most, because both macOS (WKWebView) and GNOME (WebKitGTK) run WebKit.

## Run

From the repo root:

```bash
npm run ui:check                                   # rules only, both engines, exit 1 on failure
node tools/ui-harness/shoot.mjs ./shots-after      # screenshots + metrics for review
node tools/ui-harness/shoot.mjs --check --no-shots --only=ride --engines=chromium   # quick, focused
```

- The first positional argument is the output directory. It defaults to `tools/ui-harness/shots`.
- If nothing is serving the app on :5173 or :5199, the harness starts `vite dev` itself, then stops it when finished.
- Options:

| Option | Effect |
|---|---|
| `--engines=chromium,webkit` | Which browser engines to run. |
| `--only=<substring>` | Run only routes and scenarios whose name contains the substring. |
| `--concurrency=N` | Number of pages run in parallel. |
| `--check` | Evaluate the UI rules and exit 1 on any failure. |
| `--no-shots` | Measure only, without screenshots. Also skips the fonts check. |
| `--no-fonts-check` | Skip the fonts check. |
| `--no-metrics` | Don't write the per-page measurements. |
| `--base=URL` | Use an already-running app at this URL. |
| `--repo=PATH` | Repo root to run from. |

## Mock states

The mock is `tauri-mock.js`, injected before the app loads. Pick a state with the `?mock=` query parameter:

| State | What it simulates |
|---|---|
| `normal` | Default fixtures: about 25 rides, connected devices, FTP 250. |
| `empty` | No rides and no devices. |
| `error` | Commands reject with realistic `{code, message}` errors. |
| `riding` | An active session, with live metrics every 1 s. |
| `riding-zone` | `riding` plus an active zone ride. |
| `&prereq=fail` | The Linux setup banner. |

Fixtures use a fixed seed, so screenshots are repeatable. `window.__mockEmit(event, payload)` fires backend events.

## What `--check` enforces

`checks.mjs` turns the per-page measurements into failures:
- text below 12 px;
- contrast below 4.5:1, or 3:1 for large or disabled text;
- targets below 32 px;
- controls rendered in a fallback font;
- horizontal scroll;
- content not centred, or more than 30 % empty at 1920;
- review-page cards wider than 360 px;
- ride controls off-screen;
- metric values clipped, wrapped or below their minimum size;
- any request to a host other than localhost.

`SCENARIOS` holds scripted keyboard, timing and flow checks. They use `page.clock`, never sleeps.

**Exemptions:**
- elements inside `[data-dev-only]`, which is the dev toolbar, absent from release builds;
- links inside a `<p>`, which are inline text links.

**Stable hooks** for measuring are `data-page`, `data-metric-card`, `data-metric-value="power|primary|secondary|stat"`, `data-ride-control` and `data-notice`. Keep them when changing the UI.
