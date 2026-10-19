import { describe, it, expect } from 'vitest';
import { CHART_TEXT, CHART_BG, TOOLTIP_BG, SERIES, POWER_ZONE_COLORS, HR_ZONE_COLORS, withAlpha, axisLabel, legendDefaults, tooltipDefaults } from './theme';
import { contrastRatio } from '$lib/ui/contrast';

describe('chart theme', () => {
  it('every chart text size is at least 12px', () => {
    for (const [role, t] of Object.entries(CHART_TEXT)) expect(t.fontSize, role).toBeGreaterThanOrEqual(12);
  });

  it('axis, legend and label text is >= 4.5:1 on the chart background', () => {
    for (const role of ['axis', 'axisName', 'legend', 'label'] as const) {
      expect(contrastRatio(CHART_TEXT[role].color, CHART_BG), role).toBeGreaterThanOrEqual(4.5);
    }
  });

  it('tooltip text is >= 4.5:1 on the tooltip background', () => {
    expect(contrastRatio(CHART_TEXT.tooltip.color, TOOLTIP_BG)).toBeGreaterThanOrEqual(4.5);
  });

  it('helpers carry the themed sizes', () => {
    expect(axisLabel().fontSize).toBe(12);
    expect(legendDefaults().textStyle.fontSize).toBe(13);
    expect(tooltipDefaults().textStyle.fontSize).toBe(13);
  });

  it('each series has a distinct colour except deliberate pairs', () => {
    expect(SERIES.hr).not.toBe(SERIES.power);
    expect(new Set([SERIES.power, SERIES.hr, SERIES.cadence, SERIES.speed, SERIES.cmdPower]).size).toBe(5);
  });

  it('there are 7 power zones and 5 HR zones', () => {
    expect(POWER_ZONE_COLORS).toHaveLength(7);
    expect(HR_ZONE_COLORS).toHaveLength(5);
  });

  it('withAlpha converts hex to rgba', () => {
    expect(withAlpha('#ff4d6d', 0.25)).toBe('rgba(255, 77, 109, 0.25)');
  });
});
