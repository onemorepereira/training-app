<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as echarts from 'echarts';
  import { createDisposeGuard } from '$lib/utils/disposeGuard';
  import type { PowerCurvePoint } from '$lib/tauri';
  import { SERIES, withAlpha, axisLabel, axisNameStyle, axisLine, splitLine, legendDefaults, tooltipDefaults, FONT_DATA } from '$lib/charts/theme';

  /** Durations riders compare (plus 1s as the curve's start), labelled on the log axis. */
  const KEY_DURATIONS: [number, string][] = [
    [1, '1s'],
    [5, '5s'],
    [60, '1 min'],
    [300, '5 min'],
    [1200, '20 min'],
    [3600, '1 h'],
  ];
  const KEY_LABEL = new Map(KEY_DURATIONS);

  interface OverlaySeries {
    label: string;
    data: PowerCurvePoint[];
    color: string;
  }

  interface Props {
    powerCurve: PowerCurvePoint[];
    ftp?: number | null;
    overlays?: OverlaySeries[];
  }

  let { powerCurve, ftp = null, overlays = [] }: Props = $props();

  let chartEl: HTMLDivElement;
  const guard = createDisposeGuard();
  let chart = $state<echarts.ECharts | null>(null);

  function formatDuration(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m`;
    return `${Math.floor(secs / 3600)}h`;
  }

  function getBaseOption(): echarts.EChartsOption {
    return {
      backgroundColor: 'transparent',
      animation: false,
      grid: {
        left: 56,
        right: 32,
        top: 36,
        bottom: 40,
      },
      tooltip: {
        trigger: 'axis',
        ...tooltipDefaults(),
      },
      legend: legendDefaults({ show: false, top: 4, right: 10, itemHeight: 3 }),
      xAxis: {
        type: 'log',
        min: 1,
        axisLine: axisLine(),
        axisTick: { show: false },
        axisLabel: axisLabel({
          customValues: KEY_DURATIONS.map(([d]) => d),
          formatter: (value: number) => KEY_LABEL.get(value) ?? '',
        }),
        splitLine: { show: false },
      },
      yAxis: {
        type: 'value',
        name: 'W',
        nameTextStyle: axisNameStyle(),
        min: 0,
        splitLine: splitLine(),
        axisLine: { show: false },
        axisLabel: axisLabel(),
      },
      series: [],
    };
  }

  onMount(() => {
    document.fonts.ready.then(() => {
      // Fonts may resolve after unmount: don't create a chart nobody will dispose.
      if (guard.disposed) return;
      const instance = echarts.init(chartEl, undefined, { renderer: 'canvas' });
      chart = instance;
      guard.add(() => { instance.dispose(); chart = null; });
      chart.setOption(getBaseOption());

      const observer = new ResizeObserver(() => chart?.resize());
      observer.observe(chartEl);
      guard.add(() => observer.disconnect());
    });
  });

  onDestroy(() => guard.dispose());

  $effect(() => {
    const curve = powerCurve;
    const ftpVal = ftp;
    const ovls = overlays;
    if (!chart || curve.length === 0) return;

    const data = curve.map((p) => [p.duration_secs, p.watts]);
    const allWatts = [...curve.map((p) => p.watts)];
    const allDurations = [...curve.map((p) => p.duration_secs)];

    for (const o of ovls) {
      allWatts.push(...o.data.map((p) => p.watts));
      allDurations.push(...o.data.map((p) => p.duration_secs));
    }
    if (ftpVal) allWatts.push(ftpVal);

    const rideMax = Math.max(...curve.map((p) => p.duration_secs));
    const maxWatts = Math.max(...allWatts);
    const maxDuration = Math.max(...allDurations);

    const markLine = ftpVal
      ? {
          silent: true,
          symbol: 'none',
          lineStyle: { color: SERIES.ftp, type: 'dashed' as const, width: 1.5 },
          label: {
            formatter: `FTP ${ftpVal}W`,
            color: SERIES.ftp,
            fontSize: 12,
            fontFamily: FONT_DATA,
            position: 'insideStartTop' as const,
          },
          data: [{ yAxis: ftpVal }],
        }
      : undefined;

    const series: echarts.SeriesOption[] = [
      {
        type: 'line',
        name: 'Session',
        smooth: true,
        symbol: 'circle',
        symbolSize: 6,
        lineStyle: { color: SERIES.power, width: 2 },
        itemStyle: { color: SERIES.power },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: withAlpha(SERIES.power, 0.25) },
            { offset: 1, color: withAlpha(SERIES.power, 0) },
          ]),
        },
        data,
        markLine,
      },
    ];

    for (const o of ovls) {
      series.push({
        type: 'line',
        name: o.label,
        smooth: true,
        symbol: 'none',
        lineStyle: { color: o.color, width: 1.5, type: 'dashed' },
        itemStyle: { color: o.color },
        data: o.data.map((p) => [p.duration_secs, p.watts]),
      });
    }

    const hasOverlays = ovls.length > 0;

    chart.setOption(
      {
        legend: { show: hasOverlays },
        tooltip: {
          formatter(params: unknown) {
            const items = (Array.isArray(params) ? params : [params]) as {
              seriesName: string;
              data: [number, number];
              color: string;
            }[];
            if (items.length === 0 || !items[0]?.data) return '';
            const secs = items[0].data[0];
            const lines: string[] = [`<b>${formatDuration(secs)}</b>`];
            for (const item of items) {
              if (!item?.data) continue;
              const dot = `<span style="display:inline-block;width:8px;height:8px;border-radius:50%;background:${item.color};margin-right:6px"></span>`;
              lines.push(`${dot}${item.seriesName}: <b>${item.data[1]}W</b>`);
            }
            return lines.join('<br>');
          },
        },
        // Label only the key durations the ride actually covers.
        xAxis: {
          max: maxDuration,
          axisLabel: { customValues: KEY_DURATIONS.map(([d]) => d).filter((d) => d <= rideMax) },
        },
        yAxis: { max: Math.ceil(maxWatts / 50) * 50 + 50 },
        series,
      },
      { replaceMerge: ['series'] },
    );
  });
</script>

<div class="chart-wrapper">
  <div bind:this={chartEl} class="chart"></div>
</div>

<style>
  .chart-wrapper {
    background: var(--bg-surface);
    border-radius: var(--radius-lg);
    border: 1px solid var(--border-subtle);
    padding: var(--space-md);
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
  }

  .chart {
    width: 100%;
    flex: 1;
    min-height: 0;
  }
</style>
