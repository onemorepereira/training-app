<script lang="ts">
  import { onMount } from 'svelte';
  import { initCanvas } from '$lib/retro/renderer/canvas.js';
  import { createGameLoop, type GameLoop } from '$lib/retro/gameLoop.js';
  import { CANVAS_WIDTH, CANVAS_HEIGHT } from '$lib/retro/types.js';
  import { retroMuted } from '$lib/stores/retro';
  import { sessionActive } from '$lib/stores/session';

  let canvasEl: HTMLCanvasElement;
  let loop: GameLoop | null = null;

  onMount(() => {
    const { ctx } = initCanvas(canvasEl);
    loop = createGameLoop(ctx);

    // Start immediately if session is already active
    if ($sessionActive) loop.start();

    const unsub = sessionActive.subscribe((active) => {
      if (!loop) return;
      if (active) {
        loop.start();
      } else {
        loop.stop();
      }
    });

    return () => {
      unsub();
      loop?.stop();
      loop = null;
    };
  });

  function toggleMute() {
    retroMuted.update(m => !m);
  }
</script>

<div class="retro-container">
  <canvas
    bind:this={canvasEl}
    width={CANVAS_WIDTH}
    height={CANVAS_HEIGHT}
    class="retro-canvas"
  ></canvas>
  <button
    class="mute-btn"
    onclick={toggleMute}
    aria-label={$retroMuted ? 'Unmute audio' : 'Mute audio'}
  >
    {#if $retroMuted}
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/>
        <line x1="23" y1="9" x2="17" y2="15"/>
        <line x1="17" y1="9" x2="23" y2="15"/>
      </svg>
    {:else}
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/>
        <path d="M19.07 4.93a10 10 0 0 1 0 14.14M15.54 8.46a5 5 0 0 1 0 7.07"/>
      </svg>
    {/if}
  </button>
</div>

<style>
  .retro-container {
    /* Fill width but cap height at 70vh, shrink width to preserve 16:9 */
    width: min(100%, calc(70vh * 384 / 216));
    aspect-ratio: 384 / 216;
    margin: 0 auto;
    position: relative;
    background: #000;
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  .retro-canvas {
    display: block;
    width: 100%;
    height: 100%;
    image-rendering: pixelated;
    image-rendering: crisp-edges;
  }

  .mute-btn {
    position: absolute;
    bottom: 6px;
    right: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.5);
    color: rgba(255, 255, 255, 0.5);
    cursor: pointer;
    transition: all 0.15s;
    z-index: 1;
  }

  .mute-btn:hover {
    background: rgba(0, 0, 0, 0.7);
    color: rgba(255, 255, 255, 0.9);
    border-color: rgba(255, 255, 255, 0.3);
  }
</style>
