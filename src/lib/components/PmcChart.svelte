<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';
  import type { PmcDay } from '$lib/utils/analytics';

  interface Props {
    pmcData: PmcDay[];
  }

  let { pmcData }: Props = $props();

  let chartEl: HTMLDivElement;
  let chart = $state<echarts.ECharts | null>(null);

  function getBaseOption(): echarts.EChartsOption {
    return {
      backgroundColor: 'transparent',
      animation: false,
      grid: {
        left: 50,
        right: 20,
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
        formatter(params: unknown) {
          const items = params as { seriesName: string; value: number; axisValueLabel: string; color: string }[];
          if (!Array.isArray(items) || items.length === 0) return '';
          let html = `<div style="margin-bottom:4px;font-weight:600">${items[0].axisValueLabel}</div>`;
          for (const item of items) {
            html += `<div style="display:flex;align-items:center;gap:6px">`;
            html += `<span style="width:8px;height:8px;border-radius:50%;background:${item.color};display:inline-block"></span>`;
            html += `${item.seriesName}: <b>${item.value.toFixed(1)}</b></div>`;
          }
          return html;
        },
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
      yAxis: {
        type: 'value',
        splitLine: { lineStyle: { color: 'rgba(255,255,255,0.04)' } },
        axisLine: { show: false },
        axisLabel: { color: '#70708a', fontSize: 10 },
      },
      series: [
        {
          name: 'CTL (Fitness)',
          type: 'line',
          smooth: true,
          symbol: 'none',
          lineStyle: { color: '#4a90d9', width: 2 },
          itemStyle: { color: '#4a90d9' },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: 'rgba(74, 144, 217, 0.2)' },
              { offset: 1, color: 'rgba(74, 144, 217, 0)' },
            ]),
          },
          data: [],
        },
        {
          name: 'ATL (Fatigue)',
          type: 'line',
          smooth: true,
          symbol: 'none',
          lineStyle: { color: '#ff4d6d', width: 2 },
          itemStyle: { color: '#ff4d6d' },
          data: [],
        },
        {
          name: 'TSB (Form)',
          type: 'line',
          smooth: true,
          symbol: 'none',
          lineStyle: { color: '#4caf50', width: 2 },
          itemStyle: { color: '#4caf50' },
          markLine: {
            silent: true,
            symbol: 'none',
            lineStyle: { color: '#4caf50', type: 'dashed', width: 1, opacity: 0.5 },
            data: [{ yAxis: 0 }],
          },
          data: [],
        },
      ],
    };
  }

  onMount(() => {
    let observer: ResizeObserver | undefined;
    let disposed = false;
    document.fonts.ready.then(() => {
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

  $effect(() => {
    if (!chart || pmcData.length === 0) return;

    chart.setOption({
      xAxis: {
        data: pmcData.map((d) => d.date),
        axisLabel: {
          interval: Math.max(0, Math.floor(pmcData.length / 8) - 1),
        },
      },
      series: [
        { data: pmcData.map((d) => Math.round(d.ctl * 10) / 10) },
        { data: pmcData.map((d) => Math.round(d.atl * 10) / 10) },
        { data: pmcData.map((d) => Math.round(d.tsb * 10) / 10) },
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
    display: flex;
    flex-direction: column;
  }

  .chart {
    width: 100%;
    height: 400px;
  }
</style>
