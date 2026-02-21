<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as echarts from 'echarts';

  let { power = null, ftp = 200 }: {
    power?: number | null;
    ftp?: number;
  } = $props();

  let chartEl: HTMLDivElement;
  let chart = $state<echarts.ECharts | null>(null);

  const maxPower = () => Math.max(ftp * 1.5, 500);

  function getOption(value: number) {
    return {
      series: [
        {
          type: 'gauge',
          min: 0,
          max: maxPower(),
          splitNumber: 5,
          radius: '90%',
          axisLine: {
            lineStyle: {
              width: 14,
              color: [
                [0.55 * ftp / maxPower(), '#4caf50'],
                [0.75 * ftp / maxPower(), '#ffeb3b'],
                [0.90 * ftp / maxPower(), '#ff9800'],
                [ftp / maxPower(), '#f44336'],
                [1, '#9c27b0'],
              ],
            },
          },
          pointer: {
            width: 3,
            length: '55%',
            itemStyle: { color: '#e0e0e0' },
          },
          axisTick: { show: false },
          splitLine: {
            length: 8,
            lineStyle: { color: 'rgba(255,255,255,0.12)', width: 1.5 },
          },
          axisLabel: {
            color: 'rgba(255,255,255,0.35)',
            fontSize: 10,
            distance: 15,
          },
          title: {
            show: true,
            offsetCenter: [0, '85%'],
            fontSize: 11,
            fontWeight: 600,
            color: 'rgba(255,255,255,0.3)',
            fontFamily: 'Outfit, sans-serif',
          },
          detail: {
            fontSize: 56,
            fontWeight: 'bold',
            fontFamily: 'IBM Plex Mono, monospace',
            color: '#f0f0f5',
            formatter: '{value}',
            offsetCenter: [0, '70%'],
          },
          data: [{ value, name: 'WATTS' }],
          animationDuration: 300,
          animationEasingUpdate: 'cubicOut',
        },
      ],
    };
  }

  onMount(() => {
    let observer: ResizeObserver;
    document.fonts.ready.then(() => {
      chart = echarts.init(chartEl, undefined, { renderer: 'canvas' });
      chart.setOption({
        backgroundColor: 'transparent',
        ...getOption(power ?? 0),
      });

      observer = new ResizeObserver(() => chart?.resize());
      observer.observe(chartEl);
    });

    return () => observer?.disconnect();
  });

  onDestroy(() => {
    chart?.dispose();
  });

  $effect(() => {
    if (chart) {
      chart.setOption({ series: [{ data: [{ value: power ?? 0, name: 'WATTS' }] }] });
    }
  });
</script>

<div class="gauge-wrapper">
  <div bind:this={chartEl} class="gauge"></div>
</div>

<style>
  .gauge-wrapper {
    position: absolute;
    inset: 0;
  }

  .gauge {
    width: 100%;
    height: 100%;
  }
</style>
