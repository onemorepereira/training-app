<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as echarts from 'echarts';
  import { kmhToMph } from '$lib/stores/units';
  import type { TimeseriesPoint } from '$lib/tauri';

  interface Props {
    timeseries: TimeseriesPoint[];
    smoothing?: number;
    units?: string;
  }

  let { timeseries, smoothing = 1, units = 'metric' }: Props = $props();

  let chartEl: HTMLDivElement;
  let chart: echarts.ECharts | null = null;

  const COLORS = {
    power: '#ff4d6d',
    hr: '#f44336',
    cadence: '#64b5f6',
    speed: '#4caf50',
  };

  function formatElapsed(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = Math.floor(secs % 60);
    if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
    return `${m}:${String(s).padStart(2, '0')}`;
  }

  function rollingAvg(data: (number | null)[], window: number): (number | null)[] {
    if (window <= 1) return data;
    const result: (number | null)[] = new Array(data.length);
    let sum = 0;
    let count = 0;
    for (let i = 0; i < data.length; i++) {
      if (data[i] != null) { sum += data[i]!; count++; }
      if (i >= window) {
        const old = data[i - window];
        if (old != null) { sum -= old; count--; }
      }
      result[i] = count > 0 ? Math.round(sum / count * 10) / 10 : null;
    }
    return result;
  }

  let smoothedData = $derived.by(() => {
    const imperial = units === 'imperial';
    const power = timeseries.map((p) => p.power);
    const hr = timeseries.map((p) => p.heart_rate);
    const cadence = timeseries.map((p) => p.cadence);
    const speed = timeseries.map((p) => p.speed != null ? (imperial ? kmhToMph(p.speed) : p.speed) : null);

    return {
      labels: timeseries.map((p) => formatElapsed(p.elapsed_secs)),
      power: rollingAvg(power, smoothing),
      hr: rollingAvg(hr, smoothing),
      cadence: rollingAvg(cadence, smoothing),
      speed: rollingAvg(speed, smoothing),
    };
  });

  function getBaseOption(): echarts.EChartsOption {
    const spdUnit = units === 'imperial' ? 'mph' : 'km/h';
    return {
      backgroundColor: 'transparent',
      animation: false,
      grid: {
        left: 50,
        right: 50,
        top: 40,
        bottom: 70,
      },
      legend: {
        top: 4,
        textStyle: { color: '#a0a0b8', fontSize: 12, fontFamily: 'Outfit, sans-serif' },
        itemWidth: 16,
        itemHeight: 2,
        itemGap: 16,
      },
      tooltip: {
        trigger: 'axis',
        backgroundColor: '#1c1c30',
        borderColor: 'rgba(255,255,255,0.08)',
        textStyle: { color: '#f0f0f5', fontSize: 13, fontFamily: 'IBM Plex Mono, monospace' },
      },
      dataZoom: [
        {
          type: 'slider',
          bottom: 8,
          height: 24,
          borderColor: 'rgba(255,255,255,0.06)',
          backgroundColor: 'rgba(255,255,255,0.02)',
          fillerColor: 'rgba(255,255,255,0.04)',
          handleStyle: { color: '#70708a' },
          textStyle: { color: '#70708a', fontSize: 10 },
          dataBackground: {
            lineStyle: { color: 'rgba(255,255,255,0.1)' },
            areaStyle: { color: 'rgba(255,255,255,0.03)' },
          },
        },
        { type: 'inside' },
      ],
      xAxis: {
        type: 'category',
        boundaryGap: false,
        axisLine: { lineStyle: { color: 'rgba(255,255,255,0.06)' } },
        axisTick: { show: false },
        axisLabel: { color: '#70708a', fontSize: 10 },
        data: [],
      },
      yAxis: [
        {
          type: 'value',
          name: 'W / bpm',
          nameTextStyle: { color: '#70708a', fontSize: 10 },
          min: 0,
          splitLine: { lineStyle: { color: 'rgba(255,255,255,0.04)' } },
          axisLine: { show: false },
          axisLabel: { color: '#70708a', fontSize: 10 },
        },
        {
          type: 'value',
          name: `rpm / ${spdUnit}`,
          nameTextStyle: { color: '#70708a', fontSize: 10 },
          min: 0,
          splitLine: { show: false },
          axisLine: { show: false },
          axisLabel: { color: '#70708a', fontSize: 10 },
        },
      ],
      series: [
        {
          name: 'Power',
          type: 'line',
          yAxisIndex: 0,
          smooth: true,
          symbol: 'none',
          lineStyle: { color: COLORS.power, width: 2 },
          itemStyle: { color: COLORS.power },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(255, 77, 109, 0.25)' },
              { offset: 1, color: 'rgba(255, 77, 109, 0)' },
            ]),
          },
          data: [],
        },
        {
          name: 'HR',
          type: 'line',
          yAxisIndex: 0,
          smooth: true,
          symbol: 'none',
          lineStyle: { color: COLORS.hr, width: 1.5 },
          itemStyle: { color: COLORS.hr },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(244, 67, 54, 0.15)' },
              { offset: 1, color: 'rgba(244, 67, 54, 0)' },
            ]),
          },
          data: [],
        },
        {
          name: 'Cadence',
          type: 'line',
          yAxisIndex: 1,
          smooth: true,
          symbol: 'none',
          lineStyle: { color: COLORS.cadence, width: 1.5 },
          itemStyle: { color: COLORS.cadence },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(100, 181, 246, 0.12)' },
              { offset: 1, color: 'rgba(100, 181, 246, 0)' },
            ]),
          },
          data: [],
        },
        {
          name: 'Speed',
          type: 'line',
          yAxisIndex: 1,
          smooth: true,
          symbol: 'none',
          lineStyle: { color: COLORS.speed, width: 1.5 },
          itemStyle: { color: COLORS.speed },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(76, 175, 80, 0.12)' },
              { offset: 1, color: 'rgba(76, 175, 80, 0)' },
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
    const d = smoothedData;
    if (!chart || d.labels.length === 0) return;

    const maxLeft = Math.max(
      200,
      ...d.power.map((v) => v ?? 0),
      ...d.hr.map((v) => v ?? 0),
    );
    const maxRight = Math.max(
      120,
      ...d.cadence.map((v) => v ?? 0),
      ...d.speed.map((v) => v ?? 0),
    );

    const spdUnit = units === 'imperial' ? 'mph' : 'km/h';

    chart.setOption({
      xAxis: {
        data: d.labels,
        axisLabel: {
          interval: Math.max(0, Math.floor(d.labels.length / 8) - 1),
        },
      },
      yAxis: [
        { max: Math.ceil(maxLeft / 50) * 50 },
        { max: Math.ceil(maxRight / 25) * 25, name: `rpm / ${spdUnit}` },
      ],
      series: [
        { data: d.power },
        { data: d.hr },
        { data: d.cadence },
        { data: d.speed },
      ],
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
