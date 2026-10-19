import { describe, it, expect } from 'vitest';
import * as echarts from 'echarts';
import { zoneMarkArea } from './liveChart';

describe('zoneMarkArea', () => {
  it('zone_mark_area_spans_band_bounds', () => {
    const area = zoneMarkArea({ lower: 150, upper: 180 });
    expect(area.data).toHaveLength(1);
    expect(area.data[0][0].yAxis).toBe(150);
    expect(area.data[0][1].yAxis).toBe(180);
  });

  it('zone_mark_area_empty_when_band_null', () => {
    expect(zoneMarkArea(null).data).toEqual([]);
  });

  it('echarts_ssr_mark_area_removed_after_band_cleared', () => {
    // Server-side rendering mode runs ECharts without a DOM.
    const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width: 400, height: 300 });
    chart.setOption({
      xAxis: { type: 'category', data: ['a', 'b'] },
      yAxis: { type: 'value' },
      series: [{ type: 'line', data: [160, 170] }],
    });
    chart.setOption({ series: [{ markArea: zoneMarkArea({ lower: 150, upper: 180 }) }] });
    const markAreaData = () =>
      ((chart.getOption().series as { markArea?: { data?: unknown[] } }[])[0].markArea?.data ?? []);
    expect(markAreaData()).toHaveLength(1);

    // Default (merge) setOption, as MetricsChart uses on every update
    chart.setOption({ series: [{ markArea: zoneMarkArea(null) }] });
    expect(markAreaData()).toHaveLength(0);
    chart.dispose();
  });
});
