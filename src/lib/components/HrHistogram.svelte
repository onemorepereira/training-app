<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';
  import type { TimeseriesPoint } from '$lib/tauri';
  import { HR_ZONE_COLORS, axisLabel, axisLine, splitLine, tooltipDefaults } from '$lib/charts/theme';
  import { hrZoneForBpm } from '$lib/utils/zones';

  interface Props {
    timeseries: TimeseriesPoint[];
    bucketWidth?: number;
    hrZones?: [number, number, number, number, number] | null;
  }

  let { timeseries, bucketWidth = 5, hrZones = null }: Props = $props();

  let chartEl: HTMLDivElement;
  let chart = $state<echarts.ECharts | null>(null);

  const ZONE_COLORS: readonly string[] = HR_ZONE_COLORS;

  function zoneColor(bpm: number): string {
    if (!hrZones) return ZONE_COLORS[0];
    return ZONE_COLORS[hrZoneForBpm(bpm, hrZones) - 1];
  }

  let histogram = $derived.by(() => {
    const buckets = new Map<number, number>();
    for (const pt of timeseries) {
      if (pt.heart_rate == null || pt.heart_rate <= 0) continue;
      const bucket = Math.floor(pt.heart_rate / bucketWidth) * bucketWidth;
      buckets.set(bucket, (buckets.get(bucket) ?? 0) + 1);
    }

    const sorted = [...buckets.entries()].sort((a, b) => a[0] - b[0]);
    return {
      labels: sorted.map(([hr]) => `${hr}-${hr + bucketWidth}`),
      values: sorted.map(([, count]) => count),
      colors: sorted.map(([hr]) => zoneColor(hr + bucketWidth / 2)),
    };
  });

  function formatSeconds(secs: number): string {
    const m = Math.floor(secs / 60);
    const s = secs % 60;
    if (m === 0) return `${s}s`;
    return s > 0 ? `${m}m ${s}s` : `${m}m`;
  }

  function buildOption() {
    const h = histogram;
    return {
      backgroundColor: 'transparent',
      animation: false,
      grid: { left: 56, right: 16, top: 16, bottom: 72 },
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        ...tooltipDefaults(),
        formatter(params: unknown) {
          const item = (params as { name: string; value: number }[])[0];
          if (!item) return '';
          return `${item.name} bpm<br><b>${formatSeconds(item.value)}</b>`;
        },
      },
      xAxis: {
        type: 'category',
        data: h.labels,
        axisLabel: axisLabel({ rotate: 45, hideOverlap: true }),
        axisLine: axisLine(),
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: axisLabel({ formatter: (v: number) => formatSeconds(v), }),
        splitLine: splitLine(),
        axisLine: { show: false },
      },
      series: [{
        type: 'bar',
        data: h.values.map((v, i) => ({
          value: v,
          itemStyle: { color: h.colors[i] },
        })),
        barMaxWidth: 24,
      }],
    };
  }

  onMount(() => {
    let observer: ResizeObserver | undefined;
    let disposed = false;
    document.fonts.ready.then(() => {
      if (disposed) return;
      chart = echarts.init(chartEl, undefined, { renderer: 'canvas' });
      chart.setOption(buildOption());
      observer = new ResizeObserver(() => chart?.resize());
      observer.observe(chartEl);
    });

    return () => {
      disposed = true;
      observer?.disconnect();
      chart?.dispose();
      chart = null;
    };
  });

  $effect(() => {
    histogram;
    if (!chart) return;
    chart.setOption(buildOption(), true);
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
