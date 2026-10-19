<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';
  import type { PmcDay } from '$lib/utils/analytics';
  import { SERIES, CHART_TEXT, withAlpha, axisLabel, splitLine, axisLine, legendDefaults, tooltipDefaults } from '$lib/charts/theme';

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
        left: 52,
        right: 24,
        top: 48,
        bottom: 76,
      },
      legend: legendDefaults({ top: 4, itemHeight: 3 }),
      tooltip: tooltipDefaults({
        trigger: 'axis',
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
      }),
      dataZoom: [
        {
          type: 'slider',
          bottom: 8,
          height: 24,
          borderColor: 'rgba(255,255,255,0.06)',
          backgroundColor: 'rgba(255,255,255,0.02)',
          fillerColor: 'rgba(255,255,255,0.04)',
          handleStyle: { color: CHART_TEXT.axis.color },
          textStyle: { color: CHART_TEXT.axis.color, fontSize: CHART_TEXT.axis.fontSize },
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
        axisLine: axisLine(),
        axisTick: { show: false },
        axisLabel: axisLabel(),
        data: [],
      },
      yAxis: {
        type: 'value',
        splitLine: splitLine(),
        axisLine: { show: false },
        axisLabel: axisLabel(),
      },
      series: [
        {
          name: 'CTL (Fitness)',
          type: 'line',
          smooth: true,
          symbol: 'none',
          lineStyle: { color: SERIES.ctl, width: 2 },
          itemStyle: { color: SERIES.ctl },
          areaStyle: {
            color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
              { offset: 0, color: withAlpha(SERIES.ctl, 0.2) },
              { offset: 1, color: withAlpha(SERIES.ctl, 0) },
            ]),
          },
          data: [],
        },
        {
          name: 'ATL (Fatigue)',
          type: 'line',
          smooth: true,
          symbol: 'none',
          lineStyle: { color: SERIES.atl, width: 2 },
          itemStyle: { color: SERIES.atl },
          data: [],
        },
        {
          name: 'TSB (Form)',
          type: 'line',
          smooth: true,
          symbol: 'none',
          lineStyle: { color: SERIES.speed, width: 2 },
          itemStyle: { color: SERIES.speed },
          markLine: {
            silent: true,
            symbol: 'none',
            label: { show: false },
            lineStyle: { color: SERIES.speed, type: 'dashed', width: 1, opacity: 0.5 },
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
    height: clamp(320px, 45vh, 520px);
  }
</style>
