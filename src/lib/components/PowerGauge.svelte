<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as echarts from 'echarts';
  import { createDisposeGuard } from '$lib/utils/disposeGuard';
  import { POWER_ZONE_COLORS, AXIS_LINE, axisLabel } from '$lib/charts/theme';

  /**
   * Power dial. The arc and needle are ECharts; the number is DOM text on top,
   * so it scales with the ride stage (--power-px) and is never clipped by the canvas.
   */
  let { power = null, ftp = 200 }: {
    power?: number | null;
    ftp?: number;
  } = $props();

  let chartEl: HTMLDivElement;
  const guard = createDisposeGuard();
  let chart = $state<echarts.ECharts | null>(null);

  const maxPower = () => Math.max(ftp * 1.5, 500);

  // Coggan zone tops as fractions of FTP (Z7 runs to the end of the dial).
  const ZONE_TOPS = [0.55, 0.75, 0.9, 1.05, 1.2, 1.5];

  function bands(): [number, string][] {
    const max = maxPower();
    const stops: [number, string][] = ZONE_TOPS.map((top, i) => [Math.min(1, (top * ftp) / max), POWER_ZONE_COLORS[i]]);
    stops.push([1, POWER_ZONE_COLORS[6]]);
    return stops;
  }

  function getOption(value: number) {
    return {
      series: [
        {
          type: 'gauge',
          min: 0,
          max: maxPower(),
          splitNumber: 5,
          radius: '92%',
          axisLine: { lineStyle: { width: 14, color: bands() } },
          pointer: { width: 4, length: '55%', itemStyle: { color: '#e0e0e0' } },
          axisTick: { show: false },
          splitLine: { length: 8, lineStyle: { color: AXIS_LINE, width: 1.5 } },
          // The ends (0 and max) are implied; dropping them keeps the readout clear of labels.
          axisLabel: axisLabel({ distance: 16, formatter: (v: number) => (v <= 0 || v >= maxPower() ? '' : String(v)) }),
          title: { show: false },
          detail: { show: false },
          data: [{ value }],
          animationDuration: 300,
          animationEasingUpdate: 'cubicOut',
        },
      ],
    };
  }

  onMount(() => {
    document.fonts.ready.then(() => {
      // Fonts may resolve after unmount: don't create a chart nobody will dispose.
      if (guard.disposed) return;
      const instance = echarts.init(chartEl, undefined, { renderer: 'canvas' });
      chart = instance;
      guard.add(() => { instance.dispose(); chart = null; });
      chart.setOption({
        backgroundColor: 'transparent',
        ...getOption(power ?? 0),
      });

      const observer = new ResizeObserver(() => chart?.resize());
      observer.observe(chartEl);
      guard.add(() => observer.disconnect());
    });
  });

  onDestroy(() => guard.dispose());

  $effect(() => {
    if (chart) {
      // Rebuild full option so zone color bands update when ftp changes
      chart.setOption(getOption(power ?? 0));
    }
  });
</script>

<div class="gauge-box" data-metric-card>
  <div bind:this={chartEl} class="gauge"></div>
  <div class="gauge-readout">
    <div class="gauge-value" data-metric-value="power">{power ?? '--'}{#if power != null}<span class="gauge-unit">W</span>{/if}</div>
    <div class="gauge-label">Power</div>
  </div>
</div>

<style>
  .gauge-box {
    position: relative;
    width: var(--gauge-px, 240px);
    height: var(--gauge-px, 240px);
    max-width: 100%;
    max-height: 100%;
  }

  .gauge {
    width: 100%;
    height: 100%;
  }

  /* Sits in the open bottom of the dial */
  .gauge-readout {
    position: absolute;
    left: 0;
    right: 0;
    top: 56%;
    display: flex;
    flex-direction: column;
    align-items: center;
    pointer-events: none;
  }

  .gauge-value {
    font-family: var(--font-data);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
    font-size: var(--power-px, 48px);
    line-height: 1.05;
    color: var(--text-primary);
    white-space: nowrap;
  }

  .gauge-unit {
    margin-left: 0.15em;
    font-size: 0.35em;
    font-weight: 500;
    color: var(--text-muted);
  }

  .gauge-label {
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
</style>
