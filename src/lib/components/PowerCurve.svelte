<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as echarts from 'echarts';
  import type { PowerCurvePoint } from '$lib/tauri';

  interface Props {
    powerCurve: PowerCurvePoint[];
    ftp?: number | null;
  }

  let { powerCurve, ftp = null }: Props = $props();

  let chartEl: HTMLDivElement;
  let chart: echarts.ECharts | null = null;

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
        formatter(params: unknown) {
          const p = Array.isArray(params) ? params[0] : params;
          const item = p as { data: [number, number] };
          if (!item?.data) return '';
          const [secs, watts] = item.data;
          return `${formatDuration(secs)}: <b>${watts}W</b>`;
        },
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
      series: [
        {
          type: 'line',
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
          data: [],
        },
      ],
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
    if (!chart || curve.length === 0) return;

    const data = curve.map((p) => [p.duration_secs, p.watts]);
    const maxWatts = Math.max(...curve.map((p) => p.watts), ftpVal ?? 0);
    const maxDuration = Math.max(...curve.map((p) => p.duration_secs));

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

    chart.setOption({
      xAxis: { max: maxDuration },
      yAxis: { max: Math.ceil(maxWatts / 50) * 50 + 50 },
      series: [{ data, markLine }],
    });
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
