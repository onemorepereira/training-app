/**
 * Shared ECharts styling. Charts draw text into a canvas, so the type floor and
 * contrast rules for chart text live here (checked by theme.test.ts) instead of CSS.
 * Colours mirror the tokens in app.css.
 */

export const CHART_BG = '#161625'; // --bg-surface
export const TOOLTIP_BG = '#1c1c30'; // --bg-elevated

export const CHART_TEXT = {
  axis: { fontSize: 12, color: '#8c8ca6' }, // --text-muted
  axisName: { fontSize: 12, color: '#8c8ca6' },
  legend: { fontSize: 13, color: '#a0a0b8' }, // --text-secondary
  label: { fontSize: 12, color: '#a0a0b8' },
  tooltip: { fontSize: 13, color: '#f0f0f5' }, // --text-primary
} as const;

export const FONT_SANS = "system-ui, -apple-system, 'Adwaita Sans', 'Cantarell', sans-serif";
export const FONT_DATA = "'IBM Plex Mono', ui-monospace, monospace";

export const GRID_LINE = 'rgba(255, 255, 255, 0.06)';
export const AXIS_LINE = 'rgba(255, 255, 255, 0.12)';

/** One colour per series, used by the live chart, ride detail and analytics alike. */
export const SERIES = {
  power: '#ff4d6d',
  hr: '#ff6b60',
  cadence: '#64b5f6',
  speed: '#4caf50',
  cmdPower: '#ffb74d',
  ftp: '#ffb74d',
  ctl: '#4a90d9',
  atl: '#ff4d6d',
  tsb: '#9a9ab2',
  best: '#4a90d9',
} as const;

/** Power zones 1-7 and HR zones 1-5 (canonical palette; the gauge bands use it too). */
export const POWER_ZONE_COLORS = ['#8c8ca6', '#4a90d9', '#4caf50', '#ffc107', '#ff9800', '#f44336', '#b71c1c'] as const;
export const HR_ZONE_COLORS = ['#8c8ca6', '#4a90d9', '#4caf50', '#ffc107', '#f44336'] as const;
export const POWER_ZONE_NAMES = ['Active recovery', 'Endurance', 'Tempo', 'Threshold', 'VO2max', 'Anaerobic', 'Neuromuscular'] as const;
export const HR_ZONE_NAMES = ['Recovery', 'Endurance', 'Tempo', 'Threshold', 'VO2max'] as const;

/** rgba() of a #rrggbb colour, for area fills and bands. */
export function withAlpha(hex: string, alpha: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

export function axisLabel(extra: Record<string, unknown> = {}) {
  return { color: CHART_TEXT.axis.color, fontSize: CHART_TEXT.axis.fontSize, fontFamily: FONT_DATA, ...extra };
}

export function axisNameStyle() {
  return { color: CHART_TEXT.axisName.color, fontSize: CHART_TEXT.axisName.fontSize, fontFamily: FONT_SANS };
}

export function splitLine() {
  return { lineStyle: { color: GRID_LINE } };
}

export function axisLine() {
  return { lineStyle: { color: AXIS_LINE } };
}

export function legendDefaults(extra: Record<string, unknown> = {}) {
  return {
    textStyle: { color: CHART_TEXT.legend.color, fontSize: CHART_TEXT.legend.fontSize, fontFamily: FONT_SANS },
    itemWidth: 16,
    itemHeight: 10,
    itemGap: 16,
    ...extra,
  };
}

export function tooltipDefaults(extra: Record<string, unknown> = {}) {
  return {
    backgroundColor: TOOLTIP_BG,
    borderColor: 'rgba(255, 255, 255, 0.12)',
    textStyle: { color: CHART_TEXT.tooltip.color, fontSize: CHART_TEXT.tooltip.fontSize, fontFamily: FONT_DATA },
    ...extra,
  };
}
