<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as echarts from 'echarts';
  import type { PowerCurvePoint } from '$lib/tauri';

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
        left: 50,
        right: 20,
        top: 30,
        bottom: 40,
      },
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#1c1c30',
        borderColor: 'rgba(255,255,255,0.08)',
        textStyle: { color: '#f0f0f5', fontSize: 13, fontFamily: 'IBM Plex Mono, monospace' },
      },
      legend: {
        show: false,
        top: 4,
        right: 10,
        textStyle: { color: '#70708a', fontSize: 11, fontFamily: 'IBM Plex Mono, monospace' },
        itemWidth: 16,
        itemHeight: 2,
      },
      xAxis: {
        type: 'log',
        min: 1,
        axisLine: { lineStyle: { color: 'rgba(255,255,255,0.06)' } },
        axisTick: { show: false },
        axisLabel: {
          color: '#70708a',
          fontSize: 10,
          formatter(value: number) {
            const ticks: Record<number, string> = {
              1: '1s', 2: '2s', 5: '5s', 10: '10s', 30: '30s',
              60: '1m', 120: '2m', 300: '5m', 600: '10m', 1200: '20m', 3600: '1h',
            };
            return ticks[value] ?? '';
          },
        },
        splitLine: { show: false },
      },
      yAxis: {
        type: 'value',
        name: 'W',
        nameTextStyle: { color: '#70708a', fontSize: 10 },
        min: 0,
        splitLine: { lineStyle: { color: 'rgba(255,255,255,0.04)' } },
        axisLine: { show: false },
        axisLabel: { color: '#70708a', fontSize: 10 },
      },
      series: [],
    };
  }

  onMount(() => {
    let observer: ResizeObserver;
    document.fonts.ready.then(() => {
      chart = echarts.init(chartEl, undefined, { renderer: 'canvas' });
      chart.setOption(getBaseOption());

      observer = new ResizeObserver(() => chart?.resize());
      observer.observe(chartEl);
    });

    return () => observer?.disconnect();
  });

  onDestroy(() => {
    chart?.dispose();
  });

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

    const maxWatts = Math.max(...allWatts);
    const maxDuration = Math.max(...allDurations);

    const markLine = ftpVal
      ? {
          silent: true,
          symbol: 'none',
          lineStyle: { color: '#ffa726', type: 'dashed' as const, width: 1.5 },
          label: {
            formatter: `FTP ${ftpVal}W`,
            color: '#ffa726',
            fontSize: 11,
            fontFamily: 'IBM Plex Mono, monospace',
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
        lineStyle: { color: '#ff4d6d', width: 2 },
        itemStyle: { color: '#ff4d6d' },
        areaStyle: {
          color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: 'rgba(255, 77, 109, 0.25)' },
            { offset: 1, color: 'rgba(255, 77, 109, 0)' },
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
        xAxis: { max: maxDuration },
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
