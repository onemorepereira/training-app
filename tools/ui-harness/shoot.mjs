#!/usr/bin/env node
/*
 * Visual harness for training-app: renders every route x mock state x viewport
 * in a real browser with a mocked Tauri backend, saves screenshots, console
 * errors and DOM measurements.
 *
 * Usage: node shoot.mjs [outDir] [options]
 *   outDir                default: ./shots (next to this script)
 *   --engines=a,b         chromium,webkit (default: both; webkit skipped if it can't launch)
 *   --base=URL            app URL (default: auto, starts vite if nothing is serving)
 *   --repo=PATH           repo root (default: $TRAINING_APP_REPO or this script's repo)
 *   --only=substr         only scenarios whose name contains substr (e.g. --only=history)
 *   --no-fonts-check      skip the offline-fonts comparison run
 *   --no-metrics          skip DOM measurements
 *   --concurrency=N       parallel pages (default 4)
 *   --check               evaluate the UI rules (see checks.mjs); exit 1 on any failure
 *   --no-shots            measure only, no screenshots (fast; implies --no-fonts-check)
 */
import { chromium, webkit } from 'playwright';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import { fileURLToPath } from 'node:url';
import { evaluateMetrics, SCENARIOS, summarize } from './checks.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (name, def) => {
  const a = args.find((x) => x.startsWith(`--${name}=`));
  return a ? a.slice(name.length + 3) : def;
};
const flag = (name) => args.includes(`--${name}`);
const OUT = path.resolve(args.find((a) => !a.startsWith('--')) ?? path.join(HERE, 'shots'));
const REPO = opt('repo', process.env.TRAINING_APP_REPO ?? path.resolve(HERE, '../..'));
const ENGINES_EXPLICIT = args.some((a) => a.startsWith('--engines='));
const ENGINES = opt('engines', 'chromium,webkit').split(',');
const CHECK = flag('check');
const NO_SHOTS = flag('no-shots');
const ONLY = opt('only', '');
const CONCURRENCY = Number(opt('concurrency', '4'));
const MOCK = path.join(HERE, 'tauri-mock.js');
const WEBKIT_LIBS = path.join(HERE, 'webkit-libs/lib');

const VIEWPORTS = [
  { w: 800, h: 600 },
  { w: 1280, h: 800 },
  { w: 1920, h: 1080 },
];
const STAR = 'demo-threshold-2x20';
const ROUTES = {
  ride: '/',
  devices: '/devices',
  history: '/history',
  'history-detail': `/history/${STAR}`,
  analytics: '/analytics',
  settings: '/settings',
  about: '/about',
};
// [routeSlug, state]
const MATRIX = [
  ...Object.keys(ROUTES).flatMap((r) => [[r, 'normal'], [r, 'empty']]),
  ['ride', 'riding'],
  ['ride', 'riding-zone'],
  ['devices', 'riding'],
  ['ride', 'error'],
  ['devices', 'error'],
  ['history', 'error'],
  ['history-detail', 'error'],
  ['analytics', 'error'],
];

// Interactive surfaces: [name, routeSlug, state, async (page) => void]
const INTERACTIONS = [
  ['ride-normal-zonebuilder', 'ride', 'normal', async (p) => { await p.getByRole('button', { name: 'Zone ride' }).click(); }],
  ['ride-normal-trainer-sim', 'ride', 'normal', async (p) => { await p.getByRole('radio', { name: 'SIM', exact: true }).click(); }],
  ['ride-normal-trainer-res', 'ride', 'normal', async (p) => { await p.getByRole('radio', { name: 'RES', exact: true }).click(); }],
  ['ride-riding-graphs', 'ride', 'riding', async (p) => { await p.getByRole('radio', { name: 'Graphs' }).click(); await p.waitForTimeout(2500); }],
  ['ride-riding-retro', 'ride', 'riding', async (p) => { await p.getByRole('radio', { name: 'Retro' }).click(); await p.waitForTimeout(1500); }],
  ['ride-riding-fullscreen', 'ride', 'riding', async (p) => { await p.keyboard.press('F11'); await p.waitForTimeout(3600); }],
  ['ride-riding-postride-modal', 'ride', 'riding', async (p) => { await p.getByRole('button', { name: /Stop/ }).first().click(); await p.waitForTimeout(800); }],
  ['ride-riding-paused', 'ride', 'riding', async (p) => { await p.getByRole('button', { name: /Pause/ }).click(); await p.waitForTimeout(600); }],
  ['ride-normal-nav-tooltip', 'ride', 'normal', async (p) => { await p.hover('nav a[href="/history"]'); await p.waitForTimeout(400); }],
  ['history-normal-table', 'history', 'normal', async (p) => { await p.getByRole('radio', { name: 'Table' }).click(); await p.waitForTimeout(400); }],
  ['history-detail-normal-editmodal', 'history-detail', 'normal', async (p) => { await p.getByRole('button', { name: /Edit/ }).click(); await p.waitForTimeout(500); }],
  ['devices-normal-details-modal', 'devices', 'normal', async (p) => { await p.getByRole('button', { name: 'Details', exact: true }).first().click(); await p.waitForTimeout(600); }],
  ['devices-prereqfail', 'devices', 'normal&prereq=fail', async () => {}],
  ['settings-normal-imperial', 'settings', 'normal', async (p) => { await p.getByRole('radio', { name: 'Imperial' }).click(); await p.waitForTimeout(300); }],
];

// ── Dev server ──────────────────────────────────────────────────────────
const portInUse = (port) => new Promise((res) => {
  const s = net.connect({ port, host: '127.0.0.1' }, () => { s.destroy(); res(true); });
  s.on('error', () => res(false));
});
async function isOurApp(url) {
  try {
    const r = await fetch(url, { signal: AbortSignal.timeout(3000) });
    const t = await r.text();
    return r.ok && t.includes('sveltekit');
  } catch { return false; }
}
let viteProc = null;
async function ensureServer() {
  const given = opt('base', '');
  if (given) return given;
  for (const port of [5173, 5199]) {
    const url = `http://localhost:${port}`;
    if (await isOurApp(url)) { console.log(`Using running dev server at ${url}`); return url; }
  }
  const port = (await portInUse(5173)) ? 5199 : 5173;
  console.log(`Starting vite on :${port} in ${REPO}`);
  viteProc = spawn('npx', ['vite', 'dev', '--port', String(port), '--strictPort'], { cwd: REPO, stdio: ['ignore', 'pipe', 'pipe'], detached: false });
  viteProc.stderr.on('data', (d) => process.stderr.write(`[vite] ${d}`));
  const url = `http://localhost:${port}`;
  for (let i = 0; i < 60; i++) {
    if (await isOurApp(url)) return url;
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`vite did not come up on ${url}`);
}

// ── In-page measurements ────────────────────────────────────────────────
function measure() {
  const vw = innerWidth, vh = innerHeight;
  const parse = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  const lum = ({ r, g, b }) => {
    const f = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const over = (fg, bg) => ({ r: fg.r * fg.a + bg.r * (1 - fg.a), g: fg.g * fg.a + bg.g * (1 - fg.a), b: fg.b * fg.a + bg.b * (1 - fg.a), a: 1 });
  const ratio = (a, b) => { const L1 = lum(a), L2 = lum(b); return (Math.max(L1, L2) + 0.05) / (Math.min(L1, L2) + 0.05); };
  const bodyBg = parse(getComputedStyle(document.body).backgroundColor) || { r: 15, g: 15, b: 26, a: 1 };
  function effectiveBg(el) {
    const stack = [];
    for (let e = el; e && e !== document.documentElement; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor);
      if (c && c.a > 0) { stack.push(c); if (c.a >= 1) break; }
    }
    let bg = { ...bodyBg, a: 1 };
    for (let i = stack.length - 1; i >= 0; i--) bg = over(stack[i], bg);
    return bg;
  }
  const opacityChain = (el) => { let o = 1; for (let e = el; e; e = e.parentElement) o *= parseFloat(getComputedStyle(e).opacity); return o; };
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const cs = getComputedStyle(el);
    return cs.visibility !== 'hidden' && cs.display !== 'none' && opacityChain(el) > 0.05;
  };
  const desc = (el) => {
    let s = el.tagName.toLowerCase();
    if (el.id) s += '#' + el.id;
    const cls = [...el.classList].filter((c) => !c.startsWith('svelte-')).slice(0, 2);
    if (cls.length) s += '.' + cls.join('.');
    return s;
  };
  const text = (el) => (el.innerText || el.getAttribute('aria-label') || '').trim().replace(/\s+/g, ' ').slice(0, 40);

  // Text elements (own non-empty text node)
  const fonts = {};
  const lowContrast = [];
  const all = [...document.querySelectorAll('body *')];
  for (const el of all) {
    if (el.closest('svg') || el.closest('[data-dev-only]') || ['SCRIPT', 'STYLE'].includes(el.tagName)) continue;
    const own = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
    if (!own || !visible(el)) continue;
    const cs = getComputedStyle(el);
    const fs = parseFloat(cs.fontSize);
    const key = fs.toFixed(1);
    (fonts[key] ??= { count: 0, samples: [] }).count++;
    if (fonts[key].samples.length < 4) fonts[key].samples.push(`${desc(el)} "${text(el)}"`);
    const fg = parse(cs.color);
    if (!fg) continue;
    const bg = effectiveBg(el);
    const fgA = { ...fg, a: fg.a * opacityChain(el) };
    const cr = ratio(over(fgA, bg), bg);
    const large = fs >= 24 || (fs >= 18.66 && parseInt(cs.fontWeight) >= 700);
    const disabled = !!el.closest(':disabled, [aria-disabled="true"], [data-disabled]');
    if (cr < (large || disabled ? 3 : 4.5)) lowContrast.push({ el: desc(el), text: text(el), fontPx: fs, ratio: +cr.toFixed(2), color: cs.color, bg: `rgb(${bg.r | 0},${bg.g | 0},${bg.b | 0})` });
  }

  // Interactive targets
  const smallTargets = [];
  const unlabeled = [];
  for (const el of document.querySelectorAll('button, a[href], input, select, textarea, [role="button"], [tabindex]:not([tabindex="-1"]), summary, label:has(input)')) {
    if (!visible(el) || el.closest('[data-dev-only]')) continue;
    if (el.tagName === 'A' && el.closest('p')) continue; // inline paragraph links are exempt
    // A checkbox/radio inside a label is hit through the label: measure the label's box.
    const hit = el.matches('input[type="checkbox"], input[type="radio"]') && el.closest('label') ? el.closest('label') : el;
    if (hit !== el && hit.matches('label:has(input)')) continue; // the label itself is measured separately
    const r = hit.getBoundingClientRect();
    if (Math.min(r.width, r.height) < 32) smallTargets.push({ el: desc(el), text: text(el), w: Math.round(r.width), h: Math.round(r.height) });
    if ((el.tagName === 'BUTTON' || el.tagName === 'A') && !el.innerText.trim() && !el.getAttribute('aria-label') && !el.getAttribute('title')) unlabeled.push(desc(el));
  }
  const iconOnly = [...document.querySelectorAll('button, a[href]')].filter((el) => visible(el) && !el.innerText.trim()).map((el) => ({ el: desc(el), label: el.getAttribute('aria-label') || el.getAttribute('title') || null, tooltip: el.getAttribute('title') || el.dataset.tooltip || null }));

  // Overflow
  const overflowX = [];
  for (const el of all) {
    if (!visible(el) || el.closest('svg')) continue;
    const r = el.getBoundingClientRect();
    if (r.right > vw + 1 && r.width > 0) overflowX.push({ el: desc(el), right: Math.round(r.right), width: Math.round(r.width) });
  }
  const clipped = [];
  for (const el of all) {
    if (!visible(el) || el.closest('svg') || el.tagName === 'CANVAS') continue;
    const cs = getComputedStyle(el);
    if (el.scrollWidth > el.clientWidth + 1 && (cs.overflowX === 'hidden' || cs.textOverflow === 'ellipsis') && el.clientWidth > 0)
      clipped.push({ el: desc(el), text: text(el), client: el.clientWidth, scroll: el.scrollWidth });
  }
  const scrollers = [];
  for (const el of all) {
    const cs = getComputedStyle(el);
    if (/(auto|scroll)/.test(cs.overflowY + cs.overflowX) && (el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1))
      scrollers.push({ el: desc(el), clientH: el.clientHeight, scrollH: el.scrollHeight, clientW: el.clientWidth, scrollW: el.scrollWidth });
  }
  // Content width
  const main = document.querySelector('main.content');
  const mainR = main?.getBoundingClientRect();
  const kids = main ? [...main.querySelectorAll(':scope > *, :scope > * > *')].filter(visible).slice(0, 6).map((k) => {
    const r = k.getBoundingClientRect();
    return { el: desc(k), width: Math.round(r.width), left: Math.round(r.left), maxWidth: getComputedStyle(k).maxWidth };
  }) : [];
  // Token contrast
  const root = getComputedStyle(document.documentElement);
  const tok = (n) => parse(root.getPropertyValue(n).trim().replace(/^#(..)(..)(..)$/, (_, r, g, b) => `rgb(${parseInt(r, 16)},${parseInt(g, 16)},${parseInt(b, 16)})`));
  const tokens = {};
  for (const fg of ['--text-primary', '--text-secondary', '--text-muted', '--text-faint', '--accent', '--success', '--warning', '--danger', '--info'])
    for (const bg of ['--bg-body', '--bg-surface', '--bg-elevated', '--bg-hover'])
      tokens[`${fg} on ${bg}`] = +ratio(tok(fg), tok(bg)).toFixed(2);

  // Fonts used by interactive controls (AC2)
  const controlFonts = [...document.querySelectorAll('button, input, select, textarea')].filter((el) => visible(el) && !el.closest('[data-dev-only]'))
    .map((el) => ({ el: desc(el), text: text(el), family: getComputedStyle(el).fontFamily }));
  // Ride controls must be fully inside the viewport (AC11)
  const rideControlsOut = [...document.querySelectorAll('[data-ride-control]')].filter((el) => getComputedStyle(el).display !== 'none').map((el) => {
    const r = el.getBoundingClientRect();
    return { el: desc(el), text: text(el), top: Math.round(r.top), bottom: Math.round(r.bottom), right: Math.round(r.right) };
  }).filter((r) => r.top < 0 || r.bottom > vh + 0.5 || r.right > vw + 0.5);
  const rideControlCount = document.querySelectorAll('[data-ride-control]').length;
  const rideControlsShort = [...document.querySelectorAll('[data-ride-control]')].filter(visible).map((el) => ({ el: desc(el), text: text(el), h: Math.round(el.getBoundingClientRect().height) })).filter((r) => r.h < 44);
  // Metric values: size and clipping against their card (AC12)
  const metricValues = [...document.querySelectorAll('[data-metric-value]')].filter(visible).map((el) => {
    const r = el.getBoundingClientRect();
    const box = (el.closest('[data-metric-card]') ?? el.parentElement).getBoundingClientRect();
    const clippedV = r.top < box.top - 0.5 || r.bottom > box.bottom + 0.5 || r.left < box.left - 0.5 || r.right > box.right + 0.5;
    return { kind: el.dataset.metricValue, text: text(el), fontPx: parseFloat(getComputedStyle(el).fontSize), clipped: clippedV, lines: Math.round(r.height / (parseFloat(getComputedStyle(el).lineHeight) || parseFloat(getComputedStyle(el).fontSize) * 1.2)) };
  });
  const metricCards = [...document.querySelectorAll('[data-metric-card]')].filter(visible).map((el) => ({ el: desc(el), width: Math.round(el.getBoundingClientRect().width) }));
  // Content block placement inside main (AC6)
  const pageEl = document.querySelector('[data-page]');
  let contentBlock = null;
  if (pageEl && main) {
    const pr = pageEl.getBoundingClientRect();
    const mcs = getComputedStyle(main);
    const innerL = mainR.left + parseFloat(mcs.paddingLeft), innerR = mainR.right - parseFloat(mcs.paddingRight);
    contentBlock = { page: pageEl.dataset.page, width: Math.round(pr.width), areaWidth: Math.round(innerR - innerL), gapLeft: Math.round(pr.left - innerL), gapRight: Math.round(innerR - pr.right) };
  }
  // Empty space below the last visible element of the page (AC12 fullscreen rule)
  let lastBottom = 0;
  if (pageEl) for (const el of pageEl.querySelectorAll('*')) if (visible(el)) lastBottom = Math.max(lastBottom, el.getBoundingClientRect().bottom);
  const emptyBelowPct = pageEl ? Math.max(0, Math.round(((vh - lastBottom) / vh) * 100)) : null;
  const navVisible = !!document.querySelector('nav') && visible(document.querySelector('nav'));
  const noticeVisible = [...document.querySelectorAll('[data-notice]')].some(visible);

  const fontsLoaded = { outfit: document.fonts.check('16px Outfit'), plexMono: document.fonts.check('16px "IBM Plex Mono"'), status: document.fonts.status, faces: [...document.fonts].filter((f) => f.status === 'loaded').map((f) => `${f.family} ${f.weight}`).length };
  return {
    viewport: `${vw}x${vh}`,
    docScrollWidth: document.documentElement.scrollWidth,
    horizontalScroll: document.documentElement.scrollWidth > vw,
    fontSizes: Object.fromEntries(Object.entries(fonts).sort((a, b) => parseFloat(a[0]) - parseFloat(b[0]))),
    smallestFontPx: Math.min(...Object.keys(fonts).map(Number)),
    lowContrast: lowContrast.sort((a, b) => a.ratio - b.ratio).slice(0, 40),
    lowContrastCount: lowContrast.length,
    smallTargets, unlabeled, iconOnly,
    overflowX: overflowX.slice(0, 20), clipped: clipped.slice(0, 20), scrollers,
    main: mainR ? { width: Math.round(mainR.width), contentKids: kids } : null,
    tokens, fontsLoaded,
    controlFonts, rideControlsOut, rideControlCount, rideControlsShort, metricValues, metricCards, contentBlock, emptyBelowPct, navVisible, noticeVisible,
  };
}

// ── Capture ─────────────────────────────────────────────────────────────
const FULLPAGE_CSS = `.app{height:auto!important;min-height:100vh;overflow:visible!important}main.content{overflow:visible!important}`;

async function settle(page, state) {
  await page.waitForLoadState('networkidle').catch(() => {});
  await page.evaluate(() => document.fonts.ready.then(() => true)).catch(() => {});
  await page.waitForTimeout(state.startsWith('riding') ? 2600 : 1400); // echarts animations, live ticks
}

async function shoot(page, file, { full = true } = {}) {
  await page.screenshot({ path: `${file}.png` });
  if (!full) return;
  // The app scrolls inside main.content (body never scrolls), so Playwright's
  // fullPage alone would just repeat the viewport: unclip the scroll container first.
  const tag = await page.addStyleTag({ content: FULLPAGE_CSS });
  await page.waitForTimeout(250);
  await page.screenshot({ path: `${file}-full.png`, fullPage: true });
  await tag.evaluate((n) => n.remove());
  await page.waitForTimeout(100);
}

function urlFor(base, route, state) {
  const [s, extra] = state.split('&');
  return `${base}${ROUTES[route]}?mock=${s}${extra ? '&' + extra : ''}`;
}

async function launch(engine) {
  if (engine === 'chromium') return chromium.launch();
  if (engine === 'webkit') {
    // Playwright's WebKit build is linked against Ubuntu's ICU 74 / libjpeg8. On
    // Fedora we preload copies extracted from the Ubuntu .debs (see README).
    const libs = ['libicudata.so.74', 'libicuuc.so.74', 'libicui18n.so.74', 'libjpeg.so.8'].map((l) => path.join(WEBKIT_LIBS, l));
    const env = { ...process.env };
    if (libs.every((l) => fs.existsSync(l))) env.LD_PRELOAD = libs.join(':');
    return webkit.launch({ env });
  }
  throw new Error(`unknown engine ${engine}`);
}

async function runEngine(engine, base) {
  const dir = engine === 'chromium' ? OUT : path.join(OUT, engine);
  const mdir = path.join(dir, 'metrics');
  fs.mkdirSync(mdir, { recursive: true });
  let browser;
  try { browser = await launch(engine); } catch (e) {
    console.log(`!! ${engine}: cannot launch (${e.message.split('\n')[0]})`);
    return { engine, ok: false, error: e.message.split('\n').slice(0, 3).join(' ') };
  }
  console.log(`== ${engine} ${browser.version()} -> ${dir}`);
  const consoleLog = {};
  const externalRequests = {};
  const metricsByPage = {};
  const files = [];
  const newPage = async (vp, name) => {
    const ctx = await browser.newContext({ viewport: { width: vp.w, height: vp.h }, deviceScaleFactor: 1, colorScheme: 'dark' });
    await ctx.addInitScript({ path: MOCK });
    const page = await ctx.newPage();
    const errs = (consoleLog[name] ??= []);
    page.on('console', (m) => { if (m.type() === 'error' || m.type() === 'warning') errs.push(`[${m.type()}] ${m.text()}`); });
    page.on('pageerror', (e) => errs.push(`[pageerror] ${e.message}`));
    page.on('requestfailed', (r) => { if (!r.url().includes('fonts.g')) errs.push(`[requestfailed] ${r.url()} ${r.failure()?.errorText}`); });
    const external = (externalRequests[name] ??= []);
    page.on('request', (r) => { const u = new URL(r.url()); if (/^https?:$/.test(u.protocol) && !['localhost', '127.0.0.1'].includes(u.hostname)) external.push(u.origin + u.pathname); });
    return { ctx, page };
  };

  const tasks = [];
  for (const [route, state] of MATRIX) for (const vp of VIEWPORTS)
    tasks.push({ name: `${route}-${state}-${vp.w}x${vp.h}`, route, state, vp, act: null, full: true });
  for (const [iname, route, state, act] of INTERACTIONS) for (const vp of VIEWPORTS)
    tasks.push({ name: `${iname}-${vp.w}x${vp.h}`, route, state, vp, act, full: !/modal|tooltip/.test(iname) });
  const todo = tasks.filter((t) => !ONLY || t.name.includes(ONLY));
  const worker = async () => {
    for (let t; (t = todo.shift()); ) {
      const { ctx, page } = await newPage(t.vp, t.name);
      try {
        await page.goto(urlFor(base, t.route, t.state), { waitUntil: 'domcontentloaded' });
        await settle(page, t.state);
        if (t.act) { await t.act(page); await page.waitForTimeout(400); }
        if (!NO_SHOTS) {
          await shoot(page, path.join(dir, t.name), { full: t.full });
          files.push(`${t.name}.png`); if (t.full) files.push(`${t.name}-full.png`);
        }
        if (!flag('no-metrics') || CHECK) {
          const m = await page.evaluate(measure);
          m.route = t.route; m.state = t.state; m.interaction = !!t.act;
          metricsByPage[t.name] = m;
          fs.writeFileSync(path.join(mdir, `${t.name}.json`), JSON.stringify(m, null, 1));
        }
        process.stdout.write(t.act ? '+' : '.');
      } catch (e) {
        console.log(`\n!! ${t.name}: ${e.message.split('\n')[0]}`);
        (consoleLog[t.name] ??= []).push(`[harness] ${e.message.split('\n')[0]}`);
      }
      await ctx.close();
    }
  };
  await Promise.all(Array.from({ length: CONCURRENCY }, worker));
  console.log('');

  // Offline fonts: block Google Fonts (fails fast, like DNS failure) and hang them (like a captive portal / black-holed network)
  let fontsReport = null;
  // Scripted scenario checks (keyboard, timing, flows); only in --check mode
  const scenarioFailures = {};
  if (CHECK) {
    for (const sc of SCENARIOS.filter((x) => !ONLY || x.name.includes(ONLY))) {
      const vp = sc.vp ?? { w: 1280, h: 800 };
      const { ctx, page } = await newPage(vp, `scenario:${sc.name}`);
      try {
        await page.clock.install();
        await page.goto(urlFor(base, sc.route, sc.state ?? 'normal'), { waitUntil: 'domcontentloaded' });
        await settle(page, sc.state ?? 'normal');
        const fails = await sc.run(page);
        if (fails.length) scenarioFailures[sc.name] = fails;
      } catch (e) {
        scenarioFailures[sc.name] = [`error: ${e.message.split('\n')[0]}`];
      }
      await ctx.close();
      process.stdout.write('s');
    }
    console.log('');
  }

  if (!flag('no-fonts-check') && !ONLY && !NO_SHOTS) {
    const fdir = path.join(dir, 'offline-fonts');
    fs.mkdirSync(fdir, { recursive: true });
    fontsReport = {};
    for (const mode of ['online', 'blocked', 'hang']) {
      const vp = { w: 1280, h: 800 };
      for (const route of ['ride', 'history-detail', 'settings']) {
        const name = `${route}-fonts-${mode}-1280x800`;
        const { ctx, page } = await newPage(vp, name);
        if (mode === 'blocked') await page.route(/fonts\.(googleapis|gstatic)\.com/, (r) => r.abort('internetdisconnected'));
        const held = [];
        if (mode === 'hang') await page.route(/fonts\.(googleapis|gstatic)\.com/, (r) => { held.push(r); /* held until released below */ });
        const t0 = Date.now();
        await page.goto(urlFor(base, route, route === 'ride' ? 'riding' : 'normal'), { waitUntil: 'commit' });
        // Time until the nav rail is painted & visible
        let firstUi = null;
        try {
          await page.waitForSelector('nav', { state: 'visible', timeout: 20000 });
          firstUi = Date.now() - t0;
        } catch { firstUi = '>20000'; }
        const loadMs = await page.waitForLoadState('load', { timeout: mode === 'hang' ? 8000 : 30000 }).then(() => Date.now() - t0).catch(() => `>${mode === 'hang' ? 8000 : 30000}`);
        // Release held requests (like a TCP timeout finally firing) so the screenshot can proceed.
        for (const r of held.splice(0)) await r.abort('timedout').catch(() => {});
        await page.waitForTimeout(2500);
        await page.screenshot({ path: path.join(fdir, `${name}.png`) });
        const fp = await page.evaluate(() => {
          const fcp = performance.getEntriesByName('first-contentful-paint')[0]?.startTime ?? null;
          const h = document.querySelector('h1, .gauge-value, .value');
          const cs = h && getComputedStyle(h);
          return {
            fcp: fcp && Math.round(fcp),
            outfit: document.fonts.check('16px Outfit'),
            plex: document.fonts.check('16px "IBM Plex Mono"'),
            sample: h ? { el: h.className || h.tagName, family: cs.fontFamily, width: Math.round(h.getBoundingClientRect().width) } : null,
          };
        }).catch((e) => ({ error: e.message }));
        fontsReport[name] = { firstUiMs: firstUi, loadEventMs: loadMs, ...fp };
        await ctx.close();
      }
    }
    fs.writeFileSync(path.join(fdir, 'fonts-report.json'), JSON.stringify(fontsReport, null, 1));
  }

  fs.writeFileSync(path.join(dir, 'console-errors.json'), JSON.stringify(Object.fromEntries(Object.entries(consoleLog).filter(([, v]) => v.length)), null, 1));
  await browser.close();
  return { engine, ok: true, version: browser.version(), files: files.length, consoleLog, fontsReport, metricsByPage, externalRequests, scenarioFailures };
}

const t0 = Date.now();
const base = await ensureServer();
fs.mkdirSync(OUT, { recursive: true });
const results = [];
try {
  for (const e of ENGINES) results.push(await runEngine(e, base));
} finally {
  if (viteProc) viteProc.kill();
}
// Summary
console.log(`\nDone in ${Math.round((Date.now() - t0) / 1000)}s. Output: ${OUT}`);
for (const r of results) {
  if (!r.ok) { console.log(`- ${r.engine}: FAILED to launch: ${r.error}`); continue; }
  const uniq = new Map();
  for (const [page, msgs] of Object.entries(r.consoleLog)) for (const m of msgs) {
    const k = m.replace(/\d{3,}/g, 'N').slice(0, 200);
    if (!uniq.has(k)) uniq.set(k, []);
    uniq.get(k).push(page);
  }
  console.log(`- ${r.engine} ${r.version}: ${r.files} screenshots, ${uniq.size} distinct console errors/warnings`);
  for (const [m, pages] of uniq) console.log(`    ${m}\n      on ${pages.length} page(s): ${pages.slice(0, 4).join(', ')}${pages.length > 4 ? ', ...' : ''}`);
  if (r.fontsReport) for (const [k, v] of Object.entries(r.fontsReport)) console.log(`    fonts ${k}: ${JSON.stringify(v)}`);
}

if (CHECK) {
  let failed = false;
  for (const r of results) {
    if (!r.ok) {
      if (r.engine === 'webkit' && ENGINES_EXPLICIT) { console.log('WARNING: webkit unavailable'); continue; }
      console.log(`CHECK FAILED: ${r.engine} could not launch (pass --engines=chromium to skip it explicitly)`);
      failed = true;
      continue;
    }
    const res = evaluateMetrics(r.metricsByPage, r.externalRequests);
    for (const [name, fails] of Object.entries(r.scenarioFailures)) res.push(...fails.map((f) => ({ rule: `scenario:${name}`, page: name, detail: f })));
    console.log(summarize(r.engine, res));
    fs.writeFileSync(path.join(r.engine === 'chromium' ? OUT : path.join(OUT, r.engine), 'check.json'), JSON.stringify(res, null, 1));
    if (res.length) failed = true;
  }
  process.exitCode = failed ? 1 : 0;
}
