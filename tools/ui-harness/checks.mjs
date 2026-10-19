/*
 * UI rules evaluated by `shoot.mjs --check` (spec: docs/design/ui-polish/spec.md).
 * evaluateMetrics() turns per-page DOM measurements into failures; SCENARIOS are
 * scripted keyboard/timing/flow checks that run against the mocked backend.
 */

const REVIEW_PAGES = new Set(['history', 'history-detail', 'analytics']);
const SERIF_OR_UA = /^\s*"?(arial|times|liberation|dejavu serif|serif)\b/i;

// AC12 minimums (no notice shown); 80 % while a notice is visible.
const VALUE_MIN = {
  '800x600': { power: 48, primary: 40, secondary: 28 },
  '1920x1080': { power: 112, primary: 88, secondary: 28 },
};

export function evaluateMetrics(metricsByPage, externalRequests = {}) {
  const out = [];
  const fail = (rule, page, detail) => out.push({ rule, page, detail });
  for (const [page, m] of Object.entries(metricsByPage)) {
    const vp = m.viewport;
    // AC3: text >= 12 px
    for (const [px, info] of Object.entries(m.fontSizes ?? {}))
      if (parseFloat(px) < 12) fail('AC3 text < 12px', page, `${px}px x${info.count}: ${info.samples.join(' | ')}`);
    // AC4: contrast
    for (const c of m.lowContrast ?? []) fail('AC4 contrast', page, `${c.ratio}:1 ${c.el} "${c.text}" (${c.fontPx}px)`);
    // AC5: targets
    for (const t of m.smallTargets ?? []) fail('AC5 target < 32px', page, `${t.el} "${t.text}" ${t.w}x${t.h}`);
    for (const t of m.rideControlsShort ?? []) fail('AC5 ride control < 44px', page, `${t.el} "${t.text}" h=${t.h}`);
    // AC2: control fonts
    for (const f of m.controlFonts ?? []) if (SERIF_OR_UA.test(f.family)) fail('AC2 control font', page, `${f.el} "${f.text}" ${f.family}`);
    // AC6: no horizontal page scroll
    if (m.horizontalScroll) fail('AC6 horizontal scroll', page, `doc scrollWidth ${m.docScrollWidth}`);
    // AC6: centred, <=30 % empty at 1920; review-page cards <= 360
    if (!m.contentBlock) fail('hook [data-page] missing', page, m.route);
    else if (vp === '1920x1080' && !m.interaction) {
      const b = m.contentBlock;
      const spansAll = b.width >= b.areaWidth - 2;
      if (!spansAll && Math.abs(b.gapLeft - b.gapRight) > 16) fail('AC6 not centred', page, `gaps ${b.gapLeft}/${b.gapRight}`);
      const emptyPct = Math.round(((b.areaWidth - b.width) / b.areaWidth) * 100);
      if (emptyPct > 30) fail('AC6 >30% empty', page, `${emptyPct}% (${b.width}/${b.areaWidth})`);
    }
    if (REVIEW_PAGES.has(m.route)) for (const c of m.metricCards ?? []) if (c.width > 360) fail('AC6 card > 360px', page, `${c.el} ${c.width}px`);
    // AC11: ride controls inside viewport
    if (m.route === 'ride' && m.state.startsWith('riding')) {
      if (!m.rideControlCount) fail('hook [data-ride-control] missing', page, '');
      for (const r of m.rideControlsOut ?? []) fail('AC11 ride control off-screen', page, `${r.el} "${r.text}" top ${r.top} bottom ${r.bottom}`);
    }
    // AC12: values not clipped, one line, minimum sizes
    for (const v of m.metricValues ?? []) {
      if (v.clipped) fail('AC12 value clipped', page, `${v.kind} "${v.text}"`);
      if (v.lines > 1) fail('AC12 value wraps', page, `${v.kind} "${v.text}"`);
      const mins = m.route === 'ride' && m.state.startsWith('riding') && !m.interaction ? VALUE_MIN[vp] : null;
      const min = mins?.[v.kind];
      if (min && v.fontPx < min * (m.noticeVisible ? 0.8 : 1)) fail('AC12 value too small', page, `${v.kind} ${v.fontPx}px < ${min}px`);
    }
    if (page.startsWith('ride-riding-fullscreen-1920x1080') && m.emptyBelowPct > 15) fail('AC12 empty below > 15%', page, `${m.emptyBelowPct}%`);
  }
  // AC1: no external requests anywhere
  for (const [page, urls] of Object.entries(externalRequests))
    if (urls.length) fail('AC1 external request', page, [...new Set(urls)].slice(0, 3).join(', '));
  return out;
}

export function summarize(engine, results) {
  if (!results.length) return `CHECK ${engine}: PASS (0 failures)`;
  const byRule = new Map();
  for (const r of results) {
    if (!byRule.has(r.rule)) byRule.set(r.rule, []);
    byRule.get(r.rule).push(r);
  }
  const lines = [`CHECK ${engine}: FAIL (${results.length} failures)`];
  for (const [rule, rs] of [...byRule].sort((a, b) => b[1].length - a[1].length)) {
    lines.push(`  ${String(rs.length).padStart(4)}  ${rule}`);
    for (const r of rs.slice(0, 3)) lines.push(`          ${r.page}: ${r.detail}`.slice(0, 220));
  }
  return lines.join('\n');
}

/*
 * Scripted scenarios: { name, route, state?, vp?, run(page) -> string[] failures }.
 * The clock is installed (page.clock) before navigation; use page.clock.runFor() instead of sleeps.
 */
const visibleOpacity = (loc) => loc.evaluate((el) => {
  let o = 1;
  for (let e = el; e; e = e.parentElement) o *= parseFloat(getComputedStyle(e).opacity);
  return o;
});

async function tabTo(page, predicate, max = 25, arg = null) {
  for (let i = 0; i < max; i++) {
    await page.keyboard.press('Tab');
    if (await page.evaluate(predicate, arg)) return true;
  }
  return false;
}

export const SCENARIOS = [
  {
    name: 'AC8 rail tooltips on keyboard focus',
    route: 'history',
    vp: { w: 800, h: 600 },
    async run(page) {
      const fails = [];
      const links = page.locator('nav .nav-links a');
      const n = await links.count();
      if (n !== 6) fails.push(`expected 6 nav links, found ${n}`);
      for (let i = 0; i < n; i++) {
        const href = await links.nth(i).getAttribute('href');
        const ok = await tabTo(page, (h) => document.activeElement?.getAttribute('href') === h && document.activeElement.closest('nav'), 30, href).catch(() => false);
        if (!ok) { fails.push(`could not Tab to nav link ${href}`); continue; }
        const tip = links.nth(i).locator('[role="tooltip"]');
        await page.waitForTimeout(250);
        if ((await tip.count()) === 0 || (await visibleOpacity(tip)) < 0.9) fails.push(`no visible tooltip on focus for ${href}`);
      }
      return fails;
    },
  },
  {
    name: 'AC8 sidebar shows labels; device indicator is named',
    route: 'history',
    vp: { w: 1280, h: 800 },
    async run(page) {
      const fails = [];
      const labels = await page.locator('nav .nav-links a').allInnerTexts();
      for (const want of ['Ride', 'Devices', 'History', 'Analytics', 'Settings', 'About'])
        if (!labels.some((t) => t.trim() === want)) fails.push(`no visible label "${want}"`);
      const name = await page.locator('nav a[href="/devices"].device-status, nav .device-status').first().getAttribute('aria-label');
      if (!/^(\d+ devices? connected|No devices connected)$/.test(name ?? '')) fails.push(`device indicator name "${name}"`);
      const text = await page.locator('nav .device-status').innerText();
      if (!/\d+ connected|None connected/.test(text)) fails.push(`device indicator text "${text}"`);
      return fails;
    },
  },
  {
    name: 'AC9 Esc closes the dialog and leaves the nav alone',
    route: 'history-detail',
    async run(page) {
      const fails = [];
      await page.getByRole('button', { name: /^Edit/ }).click();
      await page.waitForTimeout(300);
      if (!(await page.locator('dialog[open]').count())) return ['Edit dialog did not open'];
      await page.keyboard.press('Escape');
      await page.waitForTimeout(300);
      if (await page.locator('dialog[open]').count()) fails.push('dialog still open after Esc');
      if (!(await page.locator('nav').isVisible())) fails.push('nav hidden after Esc');
      return fails;
    },
  },
  {
    name: 'AC9 Esc in a dialog during fullscreen only closes the dialog',
    route: 'history-detail',
    async run(page) {
      const fails = [];
      const fs = () => page.evaluate(() => window.__mockFullscreen());
      await page.keyboard.press('F11');
      await page.waitForTimeout(300);
      if (!(await fs())) return ['F11 did not enter fullscreen'];
      await page.getByRole('button', { name: /^Edit/ }).click();
      await page.waitForTimeout(300);
      await page.keyboard.press('Escape');
      await page.waitForTimeout(300);
      if (await page.locator('dialog[open]').count()) fails.push('dialog still open after first Esc');
      if (!(await fs())) fails.push('first Esc (closing the dialog) also left fullscreen');
      await page.keyboard.press('Escape');
      await page.waitForTimeout(300);
      if (await fs()) fails.push('second Esc did not leave fullscreen');
      if (!(await page.locator('nav').isVisible())) fails.push('nav not restored after leaving fullscreen');
      return fails;
    },
  },
  {
    name: 'AC11 worst case: 800x600 zone ride with a notice',
    route: 'ride',
    state: 'riding-zone',
    vp: { w: 800, h: 600 },
    async run(page) {
      const fails = [];
      await page.evaluate(() => window.__mockEmit('device_reconnecting', { device_id: 'ble:C4:2F:90:11:7A:3E', device_type: 'HeartRate', attempt: 2 }));
      await page.waitForTimeout(600);
      if (!(await page.locator('[data-notice]').first().isVisible())) fails.push('reconnecting notice not shown');
      const out = await page.evaluate(() => [...document.querySelectorAll('[data-ride-control]')].map((el) => {
        const r = el.getBoundingClientRect();
        return { t: (el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 30), top: r.top, bottom: r.bottom, h: r.height };
      }).filter((r) => r.h > 0 && (r.top < 0 || r.bottom > innerHeight + 0.5)));
      for (const o of out) fails.push(`ride control off-screen: "${o.t}" bottom ${Math.round(o.bottom)}`);
      const sizes = await page.evaluate(() => [...document.querySelectorAll('[data-metric-value]')].map((el) => ({ kind: el.dataset.metricValue, px: parseFloat(getComputedStyle(el).fontSize) })));
      for (const v of sizes) {
        const min = { power: 48, primary: 40, secondary: 28 }[v.kind];
        if (min && v.px < min * 0.8) fails.push(`${v.kind} ${v.px}px < 80% of ${min}px with a notice shown`);
      }
      return fails;
    },
  },
  {
    name: 'AC13 idle values all read --',
    route: 'ride',
    state: 'empty',
    async run(page) {
      const texts = await page.evaluate(() => [...document.querySelectorAll('[data-metric-value]')].map((el) => ({ kind: el.dataset.metricValue, t: el.textContent.trim() })));
      if (!texts.length) return ['no metric values found'];
      return texts.filter((x) => x.t !== '--').map((x) => `${x.kind} shows "${x.t}" instead of --`);
    },
  },
  {
    name: 'AC10 fullscreen hint and exit button',
    route: 'ride',
    state: 'riding',
    async run(page) {
      const fails = [];
      await page.keyboard.press('F11');
      await page.waitForTimeout(200);
      const hint = page.getByText('to exit fullscreen');
      if (!(await hint.isVisible())) fails.push('hint not shown on entering fullscreen');
      await page.clock.runFor(3200);
      await page.waitForTimeout(100);
      if (await hint.isVisible()) fails.push('hint still visible after 3 s');
      const exit = page.getByRole('button', { name: 'Exit fullscreen', exact: true });
      if ((await visibleOpacity(exit)) > 0.05) fails.push('exit button visible before any pointer movement');
      await page.mouse.move(400, 300);
      await page.mouse.move(420, 320);
      await page.waitForTimeout(300);
      if ((await visibleOpacity(exit)) < 0.9) fails.push('exit button not shown after pointer movement');
      await page.clock.runFor(2700);
      await page.waitForTimeout(400);
      if ((await visibleOpacity(exit)) > 0.05) fails.push('exit button still shown 2.5 s after pointer stopped');
      const focused = await tabTo(page, () => document.activeElement?.textContent?.includes('Exit fullscreen'), 40);
      if (!focused) fails.push('exit button not reachable with Tab');
      else {
        await page.waitForTimeout(300);
        if ((await visibleOpacity(exit)) < 0.9) fails.push('exit button not visible on keyboard focus');
        await page.keyboard.press('Enter');
        await page.waitForTimeout(300);
        if (await page.evaluate(() => window.__mockFullscreen())) fails.push('exit button did not leave fullscreen');
      }
      return fails;
    },
  },
];
