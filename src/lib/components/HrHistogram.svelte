<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';
  import type { TimeseriesPoint } from '$lib/tauri';

  interface Props {
    timeseries: TimeseriesPoint[];
    bucketWidth?: number;
    hrZones?: [number, number, number, number, number] | null;
  }

  let { timeseries, bucketWidth = 5, hrZones = null }: Props = $props();

  let chartEl: HTMLDivElement;
  let chart = $state<echarts.ECharts | null>(null);

  const ZONE_COLORS = ['#70708a', '#4a90d9', '#4caf50', '#ffc107', '#f44336'];

  function zoneColor(bpm: number): string {
    if (!hrZones) return ZONE_COLORS[0];
    if (bpm < hrZones[0]) return ZONE_COLORS[0]; // Z1
    if (bpm < hrZones[1]) return ZONE_COLORS[1]; // Z2
    if (bpm < hrZones[2]) return ZONE_COLORS[2]; // Z3
    if (bpm < hrZones[3]) return ZONE_COLORS[3]; // Z4
    return ZONE_COLORS[4]; // Z5
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
      grid: { left: 50, right: 16, top: 16, bottom: 60 },
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        backgroundColor: '#1c1c30',
        borderColor: 'rgba(255,255,255,0.08)',
        textStyle: { color: '#f0f0f5', fontSize: 13, fontFamily: 'IBM Plex Mono, monospace' },
        formatter(params: unknown) {
          const item = (params as { name: string; value: number }[])[0];
          if (!item) return '';
          return `${item.name} bpm<br><b>${formatSeconds(item.value)}</b>`;
        },
      },
      xAxis: {
        type: 'category',
        data: h.labels,
        axisLabel: {
          color: '#a0a0b8',
          fontSize: 10,
          fontFamily: 'IBM Plex Mono, monospace',
          rotate: 45,
          interval: 0,
        },
        axisLine: { lineStyle: { color: 'rgba(255,255,255,0.06)' } },
        axisTick: { show: false },
      },
      yAxis: {
        type: 'value',
        axisLabel: {
          color: '#70708a',
          fontSize: 10,
          formatter: (v: number) => formatSeconds(v),
        },
        splitLine: { lineStyle: { color: 'rgba(255,255,255,0.04)' } },
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
