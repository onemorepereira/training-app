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

// Console errors that are expected in a mock state (not failures).
const EXPECTED_CONSOLE = [{ state: 'error', re: /Session rehydration failed/ }];

export function evaluateMetrics(metricsByPage, externalRequests = {}, consoleLog = {}) {
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
    for (const o of m.overflowScrollers ?? []) fail('AC6 horizontal overflow', page, `${o.el} ${o.scrollW} > ${o.clientW}`);
    // Text cut off by ellipsis/overflow (intentional previews carry data-truncate-ok)
    for (const c of m.clipped ?? []) fail('AC19/AC6 clipped text', page, `${c.el} "${c.text}" ${c.scroll} > ${c.client}`);
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
  // AC24: no unexpected console errors; a page the harness couldn't run fails
  for (const [page, msgs] of Object.entries(consoleLog)) {
    const state = metricsByPage[page]?.state ?? (page.includes('-error-') ? 'error' : '');
    for (const msg of msgs) {
      if (msg.startsWith('[warning]')) continue;
      if (EXPECTED_CONSOLE.some((e) => e.state === state && e.re.test(msg))) continue;
      fail(msg.startsWith('[harness]') ? 'harness could not run page' : 'AC24 console error', page, msg.slice(0, 200));
    }
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
 * Scripted scenarios: { name, route, state?, vp?, realClock?, run(page) -> string[] failures }.
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

/** Commands the mock backend received since the last call (window.__mockCalls is [cmd, args][]). */
async function callsSince(page, mark) {
  return page.evaluate((m) => (window.__mockCalls || []).slice(m).map((c) => c[0]), mark);
}
const mark = (page) => page.evaluate(() => (window.__mockCalls || []).length);

/** Run steps; each step clicks/acts then expects some backend commands. */
async function walk(page, steps) {
  const fails = [];
  for (const step of steps) {
    const m = await mark(page);
    try {
      await step.act(page);
    } catch (e) {
      fails.push(`${step.name}: ${e.message.split('\n')[0]}`);
      break;
    }
    // Poll for the expected commands (up to `wait`, default 3 s) instead of a fixed sleep.
    const deadline = Date.now() + (step.wait ?? 3000);
    let got = await callsSince(page, m);
    while ((step.expect ?? []).some((c) => !got.includes(c)) && Date.now() < deadline) {
      await page.waitForTimeout(100);
      got = await callsSince(page, m);
    }
    await page.waitForTimeout(300); // let the UI settle before step.check
    for (const cmd of step.expect ?? []) if (!got.includes(cmd)) fails.push(`${step.name}: expected ${cmd}, got [${got.join(', ')}]`);
    if (step.check) {
      const extra = await step.check(page);
      if (extra) fails.push(`${step.name}: ${extra}`);
    }
  }
  return fails;
}

export const SCENARIOS = [
  {
    name: 'AC24 flow: start, pause, resume, stop, post-ride Done',
    route: 'ride',
    async run(page) {
      const controls = page.locator('section[aria-label="Ride controls"]');
      const btn = (name) => (name === 'Done' ? page : controls).getByRole('button', { name, exact: true });
      return walk(page, [
        { name: 'start', act: () => btn('Start').click(), expect: ['start_session'] },
        { name: 'pause', act: () => btn('Pause').click(), expect: ['pause_session'] },
        { name: 'resume', act: () => btn('Resume').click(), expect: ['resume_session'] },
        {
          name: 'stop',
          act: () => btn('Stop').click(),
          expect: ['stop_session'],
          wait: 800,
          check: async (p) => ((await p.getByRole('dialog', { name: 'Ride saved' }).isVisible()) ? null : 'post-ride dialog not shown'),
        },
        {
          name: 'done',
          act: () => btn('Done').click(),
          check: async (p) => ((await p.locator('dialog[open]').count()) ? 'dialog still open after Done' : null),
        },
      ]);
    },
  },
  {
    name: 'AC24 flow: trainer mode and adjust',
    route: 'ride',
    async run(page) {
      return walk(page, [
        { name: 'SIM mode', act: (p) => p.getByRole('radio', { name: 'SIM', exact: true }).click(), expect: ['set_trainer_simulation'] },
        { name: 'grade +1', act: (p) => p.getByRole('button', { name: 'Increase grade by 1 percent' }).click(), expect: ['set_trainer_simulation'] },
        { name: 'ERG mode', act: (p) => p.getByRole('radio', { name: 'ERG', exact: true }).click(), expect: ['set_trainer_power'] },
        { name: 'ERG +25', act: (p) => p.getByRole('button', { name: 'Increase target by 25 watts' }).click(), expect: ['set_trainer_power'] },
      ]);
    },
  },
  {
    name: 'AC24 flow: zone ride start from the builder',
    route: 'ride',
    async run(page) {
      return walk(page, [
        {
          name: 'open builder',
          act: (p) => p.getByRole('button', { name: 'Zone ride', exact: true }).click(),
          check: async (p) => ((await p.getByRole('dialog', { name: 'Start a zone ride' }).isVisible()) ? null : 'zone builder dialog not shown'),
        },
        { name: 'pick Z2', act: (p) => p.getByRole('radio', { name: /^Z2/ }).click() },
        { name: 'start', act: (p) => p.getByRole('button', { name: 'Start zone ride', exact: true }).click(), expect: ['start_zone_control'], wait: 800 },
      ]);
    },
  },
  {
    name: 'AC24 flow: devices scan, details, disconnect, connect',
    route: 'devices',
    async run(page) {
      return walk(page, [
        {
          name: 'scan',
          act: async (p) => {
            // Wait for the initial load to finish so the button is the settled, enabled "Scan for devices".
            const scan = p.getByRole('button', { name: 'Scan for devices', exact: true });
            await scan.waitFor({ state: 'visible' });
            await p.waitForFunction(() => [...document.querySelectorAll('button')].some((b) => b.textContent.trim() === 'Scan for devices' && !b.disabled));
            await scan.click();
          },
          expect: ['scan_devices'],
        },
        {
          name: 'details',
          act: (p) => p.getByRole('button', { name: /^Details/ }).first().click(),
          expect: ['get_device_details'],
          wait: 800,
          check: async (p) => ((await p.locator('dialog[open]').count()) ? null : 'details dialog not shown'),
        },
        { name: 'close details', act: (p) => p.keyboard.press('Escape') },
        { name: 'disconnect', act: (p) => p.getByRole('button', { name: 'Disconnect', exact: true }).first().click(), expect: ['disconnect_device'], wait: 800 },
        { name: 'connect', act: (p) => p.getByRole('button', { name: 'Connect', exact: true }).first().click(), expect: ['connect_device'], wait: 1200 },
      ]);
    },
  },
  {
    name: 'AC24 flow: open a ride, edit and save, export',
    route: 'history',
    async run(page) {
      return walk(page, [
        {
          name: 'open ride',
          act: (p) => p.locator('main a[href^="/history/"]').first().click(),
          expect: ['get_session'],
          wait: 1500,
          check: async (p) => (/\/history\/[^/]+$/.test(new URL(p.url()).pathname) ? null : `still at ${p.url()}`),
        },
        {
          name: 'edit',
          act: (p) => p.getByRole('button', { name: /^Edit/ }).click(),
          check: async (p) => ((await p.getByRole('dialog', { name: 'Edit ride' }).isVisible()) ? null : 'edit dialog not shown'),
        },
        { name: 'rename', act: (p) => p.getByRole('dialog').getByRole('textbox').first().fill('Harness edit') },
        { name: 'save', act: (p) => p.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click(), expect: ['update_session_metadata'], wait: 800 },
        { name: 'export', act: (p) => p.getByRole('button', { name: 'Export FIT', exact: true }).click(), expect: ['export_session_fit'], wait: 800 },
      ]);
    },
  },
  {
    name: 'AC24/AC18 flow: settings save bar, discard and save',
    route: 'settings',
    vp: { w: 800, h: 600 },
    async run(page) {
      const bar = (p) => p.getByText('Unsaved changes');
      return walk(page, [
        {
          name: 'clean form has no bar',
          act: async () => {},
          check: async (p) => ((await bar(p).isVisible()) ? 'save bar visible with no changes' : null),
        },
        {
          name: 'change units',
          act: (p) => p.getByRole('radio', { name: 'Imperial' }).click(),
          check: async (p) => {
            if (!(await bar(p).isVisible())) return 'save bar not shown after a change';
            const b = await bar(p).boundingBox();
            return b && b.y + b.height <= 600 ? null : 'save bar not inside the 800x600 viewport';
          },
        },
        {
          name: 'discard',
          act: (p) => p.getByRole('button', { name: 'Discard', exact: true }).click(),
          check: async (p) => ((await bar(p).isVisible()) ? 'save bar still shown after Discard' : null),
        },
        { name: 'change again', act: (p) => p.getByRole('radio', { name: 'Imperial' }).click() },
        {
          name: 'save',
          act: (p) => p.getByRole('button', { name: 'Save', exact: true }).click(),
          expect: ['save_user_config'],
          wait: 800,
          check: async (p) => ((await p.getByText('Settings saved').isVisible()) ? null : 'no "Settings saved" confirmation'),
        },
        {
          name: 'confirmation hides',
          act: (p) => p.clock.runFor(3200),
          wait: 400,
          check: async (p) => ((await p.getByText('Settings saved').isVisible()) ? 'confirmation still visible after 3 s' : null),
        },
      ]);
    },
  },
  {
    name: 'AC24 flow: stop a running zone ride',
    route: 'ride',
    state: 'riding-zone',
    async run(page) {
      return walk(page, [
        { name: 'stop zone ride', act: (p) => p.getByRole('button', { name: 'Stop zone ride', exact: true }).click(), expect: ['stop_zone_control'], wait: 800 },
      ]);
    },
  },
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
    name: 'AC1 hanging network: UI paints within 1 s, no external requests',
    route: 'history-detail',
    realClock: true, // paint timing entries need the real clock
    async run(page) {
      // Every non-local request hangs forever (captive portal / black-holed DNS).
      const attempted = [];
      await page.route((url) => !['localhost', '127.0.0.1'].includes(url.hostname), (route) => { attempted.push(route.request().url()); });
      await page.reload({ waitUntil: 'commit' });
      const ok = await page.locator('nav').waitFor({ state: 'visible', timeout: 5000 }).then(() => true, () => false);
      // WebKit can record the paint entry a frame after the nav is visible; wait for it, then judge its timestamp.
      const fcp = await page
        .waitForFunction(() => performance.getEntriesByName('first-contentful-paint')[0]?.startTime ?? null, null, { timeout: 3000 })
        .then((h) => h.jsonValue(), () => null);
      const fails = [];
      if (!ok) fails.push('nav not visible within 5 s');
      if (fcp == null || fcp > 1000) fails.push(`first contentful paint ${fcp == null ? 'missing' : Math.round(fcp) + ' ms'} (> 1000 ms)`);
      if (attempted.length) fails.push(`external requests: ${[...new Set(attempted)].slice(0, 3).join(', ')}`);
      return fails;
    },
  },
  {
    name: 'AC20 history table fits 800x600 with a readable title and sort icon',
    route: 'history',
    vp: { w: 800, h: 600 },
    async run(page) {
      await page.getByRole('radio', { name: 'Table' }).click();
      await page.waitForTimeout(400);
      return page.evaluate(() => {
        const fails = [];
        const table = document.querySelector('main table');
        if (!table) return ['no table'];
        const wrap = table.parentElement;
        if (wrap.scrollWidth > wrap.clientWidth + 1) fails.push(`table overflows ${wrap.scrollWidth} > ${wrap.clientWidth}`);
        const title = table.querySelector('tbody tr td a[href^="/history/"]')?.closest('td');
        if (!title) fails.push('no title cell');
        else {
          const ch = parseFloat(getComputedStyle(title).fontSize) * 0.5; // ~1ch for a proportional font
          if (title.clientWidth < 24 * ch) fails.push(`title column ${title.clientWidth}px < 24ch`);
        }
        const icon = table.querySelector('thead svg');
        if (!icon) fails.push('no sort icon');
        else if (icon.getBoundingClientRect().width < 12) fails.push(`sort icon ${icon.getBoundingClientRect().width}px < 12px`);
        return fails;
      });
    },
  },
  {
    name: 'AC17 Esc closes the zone builder and the post-ride dialog',
    route: 'ride',
    async run(page) {
      const fails = [];
      await page.getByRole('button', { name: 'Zone ride', exact: true }).click();
      await page.waitForTimeout(300);
      await page.keyboard.press('Escape');
      await page.waitForTimeout(300);
      if (await page.locator('dialog[open]').count()) fails.push('zone builder still open after Esc');
      const controls = page.locator('section[aria-label="Ride controls"]');
      await controls.getByRole('button', { name: 'Start', exact: true }).click();
      await page.waitForTimeout(400);
      await controls.getByRole('button', { name: 'Stop', exact: true }).click();
      await page.waitForTimeout(800);
      if (!(await page.locator('dialog[open]').count())) return [...fails, 'post-ride dialog did not open'];
      await page.keyboard.press('Escape');
      await page.waitForTimeout(300);
      if (await page.locator('dialog[open]').count()) fails.push('post-ride dialog still open after Esc');
      if (!(await page.locator('nav').isVisible())) fails.push('nav hidden after Esc');
      return fails;
    },
  },
  {
    name: 'Arrow keys on a segmented control do not change the trainer',
    route: 'ride',
    async run(page) {
      return walk(page, [
        {
          name: 'arrow on view switcher',
          act: async (p) => {
            await p.getByRole('radio', { name: 'Gauges' }).focus();
            p.__mark = await mark(p);
            await p.keyboard.press('ArrowRight');
            await p.keyboard.press('ArrowDown');
          },
          check: async (p) => {
            const got = await callsSince(p, p.__mark);
            return got.some((c) => c.startsWith('set_trainer')) ? `trainer command sent: ${got.join(', ')}` : null;
          },
        },
        {
          name: 'arrow on trainer mode',
          act: async (p) => {
            await p.getByRole('radio', { name: 'ERG', exact: true }).focus();
            p.__mark = await mark(p);
            await p.keyboard.press('ArrowUp');
          },
          check: async (p) => {
            const calls = await callsSince(p, p.__mark);
            // Changing mode sends one command for the new mode, but no extra adjust on top of it.
            return calls.filter((c) => c.startsWith('set_trainer')).length > 1 ? `extra trainer adjust: ${calls.join(', ')}` : null;
          },
        },
      ]);
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
