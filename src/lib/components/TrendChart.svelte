<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';
  import { axisLabel, axisNameStyle, splitLine, axisLine, legendDefaults, tooltipDefaults } from '$lib/charts/theme';

  interface Props {
    labels: string[];
    barData: number[];
    barLabel: string;
    barColor: string;
    barUnit?: string;
    lineData?: (number | null)[];
    lineLabel?: string;
    lineColor?: string;
    lineUnit?: string;
  }

  let {
    labels,
    barData,
    barLabel,
    barColor,
    barUnit = '',
    lineData,
    lineLabel,
    lineColor,
    lineUnit = '',
  }: Props = $props();

  let chartEl: HTMLDivElement;
  let chart = $state<echarts.ECharts | null>(null);

  const hasLine = $derived(lineData != null && lineLabel != null && lineColor != null);

  function getBaseOption(): echarts.EChartsOption {
    const yAxes: echarts.EChartsOption['yAxis'] = [
      {
        type: 'value',
        name: barUnit ? `${barLabel} (${barUnit})` : barLabel,
        nameTextStyle: axisNameStyle(),
        min: 0,
        splitLine: splitLine(),
        axisLine: { show: false },
        axisLabel: axisLabel(),
      },
    ];

    if (hasLine) {
      yAxes.push({
        type: 'value',
        name: lineUnit ? `${lineLabel} (${lineUnit})` : lineLabel,
        nameTextStyle: axisNameStyle(),
        min: 0,
        splitLine: { show: false },
        axisLine: { show: false },
        axisLabel: axisLabel(),
      });
    }

    const series: echarts.SeriesOption[] = [
      {
        name: barLabel,
        type: 'bar',
        yAxisIndex: 0,
        itemStyle: {
          color: barColor,
          opacity: 0.7,
          borderRadius: [4, 4, 0, 0],
        },
        data: [],
      },
    ];

    if (hasLine) {
      series.push({
        name: lineLabel,
        type: 'line',
        yAxisIndex: 1,
        smooth: true,
        symbol: 'circle',
        symbolSize: 4,
        lineStyle: { color: lineColor, width: 2 },
        itemStyle: { color: lineColor },
        data: [],
      });
    }

    return {
      backgroundColor: 'transparent',
      animation: false,
      grid: {
        left: 12,
        right: 12,
        top: 52,
        bottom: 8,
        containLabel: true,
      },
      legend: legendDefaults({ top: 4, itemHeight: 3 }),
      tooltip: tooltipDefaults({ trigger: 'axis' }),
      xAxis: {
        type: 'category',
        axisLine: axisLine(),
        axisTick: { show: false },
        axisLabel: axisLabel({ rotate: labels.length > 12 ? 45 : 0 }),
        data: [],
      },
      yAxis: yAxes,
      series,
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
    if (!chart || labels.length === 0) return;

    const seriesData: echarts.SeriesOption[] = [
      { data: barData },
    ];
    if (hasLine) {
      seriesData.push({ data: lineData!.map((v) => v != null ? Math.round(v * 10) / 10 : null) });
    }

    chart.setOption({
      xAxis: { data: labels },
      series: seriesData,
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
    height: clamp(280px, 36vh, 420px);
  }
</style>
