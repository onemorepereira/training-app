<script lang="ts">
  import type { Snippet } from 'svelte';
  import { rideLayout, type RideLayout } from '$lib/utils/rideLayout';

  /**
   * The ride screen's flexible middle: measures itself and exposes the sizes from
   * rideLayout() as CSS variables (--power-px, --gauge-px, --value-px,
   * --small-value-px, --strip-px, --stage-w, --stage-h) for the views inside.
   */
  let { children }: { children: Snippet<[RideLayout]> } = $props();

  let el: HTMLDivElement;
  let w = $state(0);
  let h = $state(0);
  let layout = $derived(rideLayout(w, h));

  $effect(() => {
    let frame = 0;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      cancelAnimationFrame(frame);
      // Coalesce bursts (window drags) into one update per frame; only write on change.
      frame = requestAnimationFrame(() => {
        const nw = Math.round(r.width);
        const nh = Math.round(r.height);
        if (nw !== w) w = nw;
        if (nh !== h) h = nh;
      });
    });
    ro.observe(el);
    return () => {
      cancelAnimationFrame(frame);
      ro.disconnect();
    };
  });
</script>

<div
  bind:this={el}
  class="stage stage-{layout.mode}"
  style="--stage-w: {w}px; --stage-h: {h}px; --power-px: {layout.powerPx}px; --gauge-px: {layout.gaugePx}px; --value-px: {layout.valuePx}px; --small-value-px: {layout.smallValuePx}px; --strip-px: {layout.stripPx}px;"
>
  {@render children(layout)}
</div>

<style>
  .stage {
    flex: 1;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }
</style>
