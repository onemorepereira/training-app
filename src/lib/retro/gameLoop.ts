/**
 * Game loop — drives the retro ride animation via requestAnimationFrame.
 * Reads sensor stores synchronously each frame, updates game state, renders.
 */

import { get } from 'svelte/store';
import { currentPower, currentHR, currentCadence, currentSpeed, liveMetrics } from '$lib/stores/sensor';
import { retroMuted, retroScore } from '$lib/stores/retro';
import { clearCanvas } from './renderer/canvas.js';
import { renderRoad, getPlayerX, getRoadY } from './renderer/roadRenderer.js';
import { renderCyclist } from './renderer/cyclistRenderer.js';
import { renderNpcs } from './renderer/npcRenderer.js';
import { renderScenery } from './renderer/sceneryRenderer.js';
import { renderHud } from './renderer/hudRenderer.js';
import { renderEnvironment } from './renderer/environmentRenderer.js';
import { renderPopups } from './renderer/popupRenderer.js';
import { createGameState, updateGameState } from './state/gameState.js';
import { createEventState, detectEvents } from './state/eventDetector.js';
import { createScoreState, updateScore } from './state/scoreTracker.js';
import { createNpcManagerState, updateNpcs } from './state/npcManager.js';
import { setMuted } from './audio/audioEngine.js';
import { createEffectState, updateEffects, applyPreEffects, applyPostEffects } from './sprites/effects.js';
import type { SensorSnapshot } from './types.js';

/** Maximum delta time in seconds to prevent physics jumps */
const MAX_DT = 0.05;

export interface GameLoop {
  start: () => void;
  stop: () => void;
}

/**
 * Create a game loop attached to the given canvas context.
 */
export function createGameLoop(ctx: CanvasRenderingContext2D): GameLoop {
  let state = createGameState();
  let eventState = createEventState();
  let scoreState = createScoreState();
  let npcManager = createNpcManagerState();
  let effectState = createEffectState();
  let rafId: number | null = null;
  let lastTime = 0;

  function readSensors(): SensorSnapshot {
    const metrics = get(liveMetrics);
    return {
      power: get(currentPower),
      hr: get(currentHR),
      cadence: get(currentCadence),
      speed: get(currentSpeed),
      powerZone: metrics?.power_zone ?? null,
      elapsedSecs: metrics?.elapsed_secs ?? 0,
      tss: metrics?.tss ?? null,
    };
  }

  function frame(timestamp: number): void {
    if (lastTime === 0) lastTime = timestamp;

    const rawDt = (timestamp - lastTime) / 1000;
    const dt = Math.min(rawDt, MAX_DT);
    lastTime = timestamp;

    // Sync mute state from store
    const muted = get(retroMuted);
    setMuted(muted);
    state.muted = muted;

    // Read sensors
    const sensors = readSensors();

    // Update state
    updateGameState(state, sensors, dt);

    // Detect events (streaks, milestones, PRs)
    const newPopups = detectEvents(state, eventState, dt);
    state.popups.push(...newPopups);

    // Update NPCs
    const npcPopups = updateNpcs(state.npcs, npcManager, sensors.speed ?? 0, dt);
    state.popups.push(...npcPopups);

    // Update score
    updateScore(state, scoreState);
    retroScore.set(Math.floor(state.score));

    // Update visual effects
    updateEffects(effectState, sensors.powerZone ?? 0, dt);

    // Render with effects
    clearCanvas(ctx, '#000000');
    applyPreEffects(ctx, effectState);

    // Climb tilt: rotate road, NPCs, cyclist around player position
    const maxTiltRad = (4 * Math.PI) / 180; // 4° max
    const tiltAngle = -state.climbGrade * maxTiltRad; // negative = road tilts up-right
    const pivotX = getPlayerX();
    const pivotY = getRoadY();

    ctx.save();
    ctx.translate(pivotX, pivotY);
    ctx.rotate(tiltAngle);
    ctx.translate(-pivotX, -pivotY);

    renderRoad(ctx, state.roadOffset, state.climbGrade);
    renderScenery(ctx, state.roadOffset);
    renderNpcs(ctx, state.npcs);
    renderCyclist(ctx, state.cyclist, sensors.powerZone ?? 0);

    ctx.restore();

    // Environment, popups, HUD rendered level (no tilt)
    renderEnvironment(ctx, state);
    renderPopups(ctx, state.popups, dt);
    renderHud(ctx, state);
    applyPostEffects(ctx, effectState);

    rafId = requestAnimationFrame(frame);
  }

  return {
    start() {
      if (rafId !== null) return;
      lastTime = 0;
      state = createGameState();
      eventState = createEventState();
      scoreState = createScoreState();
      npcManager = createNpcManagerState();
      effectState = createEffectState();
      rafId = requestAnimationFrame(frame);
    },
    stop() {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
    },
  };
}
