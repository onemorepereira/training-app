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
export const SCENARIOS = [];
