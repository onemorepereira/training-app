<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';
  import { metricHistory } from '$lib/stores/sensor';
  import { get } from 'svelte/store';
  import { unitSystem, kmhToMph, speedUnit } from '$lib/stores/units';
  import { zoneMarkArea } from '$lib/utils/liveChart';

  let {
    zoneBand = null,
    commandedPowerSeries = null,
  }: {
    zoneBand?: { lower: number; upper: number } | null;
    commandedPowerSeries?: (number | null)[] | null;
  } = $props();

  let chartEl: HTMLDivElement;
  let chart = $state<echarts.ECharts | null>(null);

  const COLORS = {
    power: '#ff4d6d',
    hr: '#ffa726',
    cadence: '#64b5f6',
    speed: '#4caf50',
  };

  function formatRelativeTime(ts: number, now: number): string {
    const secsAgo = Math.round((now - ts) / 1000);
    const m = Math.floor(secsAgo / 60);
    const s = secsAgo % 60;
    return `-${m}:${String(s).padStart(2, '0')}`;
  }

  function getBaseOption(): echarts.EChartsOption {
    return {
      backgroundColor: 'transparent',
      animation: false,
      grid: {
        left: 50,
        right: 50,
        top: 40,
        bottom: 30,
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
          max: 500,
          splitLine: { lineStyle: { color: 'rgba(255,255,255,0.04)' } },
          axisLine: { show: false },
          axisLabel: { color: '#70708a', fontSize: 10 },
        },
        {
          type: 'value',
          name: `rpm / ${get(unitSystem) === 'imperial' ? 'mph' : 'km/h'}`,
          nameTextStyle: { color: '#70708a', fontSize: 10 },
          min: 0,
          max: 150,
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
              { offset: 0, color: 'rgba(255, 167, 38, 0.15)' },
              { offset: 1, color: 'rgba(255, 167, 38, 0)' },
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
        {
          name: 'Cmd Power',
          type: 'line',
          yAxisIndex: 0,
          smooth: false,
          symbol: 'none',
          lineStyle: { color: '#ff9800', width: 2, type: 'dashed' },
          itemStyle: { color: '#ff9800' },
          data: [],
        },
      ],
    };
  }

  onMount(() => {
    let observer: ResizeObserver | undefined;
    let disposed = false;

    // Use rAF to ensure the container has been laid out before initializing
    // echarts. The previous `document.fonts.ready` approach could fire before
    // the browser had assigned dimensions to the container, giving echarts a
    // 0×0 canvas that never painted.
    requestAnimationFrame(() => {
      if (disposed) return;
      chart = echarts.init(chartEl, undefined, { renderer: 'canvas' });
      chart.setOption(getBaseOption());

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

  // Keep axis label in sync with unit system, even with no data
  $effect(() => {
    const units = $unitSystem;
    if (!chart) return;
    const rightAxisLabel = `rpm / ${units === 'imperial' ? 'mph' : 'km/h'}`;
    chart.setOption({
      yAxis: [{}, { name: rightAxisLabel }],
    });
  });

  $effect(() => {
    const history = $metricHistory;
    const units = $unitSystem;
    if (!chart || history.length === 0) return;

    // Sizing is handled by the ResizeObserver (its first observation fires once
    // the container has a size), so no per-update resize() here.

    const now = history[history.length - 1].t;
    const labels = history.map((e) => formatRelativeTime(e.t, now));

    const convertSpeed = (v: number | null) =>
      v != null ? (units === 'imperial' ? kmhToMph(v) : v) : null;

    const speedData = history.map((e) => convertSpeed(e.speed));
    const maxPower = Math.max(200, ...history.map((e) => e.power ?? 0), ...history.map((e) => e.hr ?? 0));
    const maxRight = Math.max(120, ...history.map((e) => e.cadence ?? 0), ...speedData.map((v) => v ?? 0));

    // Always send markArea: omitting it would keep a stale band under merge mode.
    const powerSeriesOpt = { data: history.map((e) => e.power), markArea: zoneMarkArea(zoneBand) };

    const cmdData = commandedPowerSeries && commandedPowerSeries.length > 0
      ? commandedPowerSeries
      : history.map(() => null);

    chart.setOption({
      xAxis: {
        data: labels,
        axisLabel: {
          interval: Math.max(0, Math.floor(history.length / 6) - 1),
        },
      },
      yAxis: [
        { max: Math.ceil(maxPower / 50) * 50 },
        { max: Math.ceil(maxRight / 25) * 25 },
      ],
      series: [
        powerSeriesOpt,
        { data: history.map((e) => e.hr) },
        { data: history.map((e) => e.cadence) },
        { data: speedData },
        { data: cmdData },
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
