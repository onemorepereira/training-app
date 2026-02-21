<script lang="ts">
  import { onMount } from 'svelte';
  import * as echarts from 'echarts';

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
        nameTextStyle: { color: '#70708a', fontSize: 10 },
        min: 0,
        splitLine: { lineStyle: { color: 'rgba(255,255,255,0.04)' } },
        axisLine: { show: false },
        axisLabel: { color: '#70708a', fontSize: 10 },
      },
    ];

    if (hasLine) {
      yAxes.push({
        type: 'value',
        name: lineUnit ? `${lineLabel} (${lineUnit})` : lineLabel,
        nameTextStyle: { color: '#70708a', fontSize: 10 },
        min: 0,
        splitLine: { show: false },
        axisLine: { show: false },
        axisLabel: { color: '#70708a', fontSize: 10 },
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
        left: 50,
        right: hasLine ? 50 : 20,
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
        axisLine: { lineStyle: { color: 'rgba(255,255,255,0.06)' } },
        axisTick: { show: false },
        axisLabel: { color: '#70708a', fontSize: 10, rotate: labels.length > 12 ? 45 : 0 },
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
    height: 300px;
  }
</style>
