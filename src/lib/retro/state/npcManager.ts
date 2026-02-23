/**
 * NPC manager — spawn/despawn NPC cyclists, track relative position, detect passing.
 */

import type { NpcState, PopupMessage } from '../types.js';

const MAX_NPCS = 3;
const SPAWN_MIN_INTERVAL = 30; // seconds
const SPAWN_MAX_INTERVAL = 90;
const NPC_MIN_SPEED = 15; // km/h
const NPC_MAX_SPEED = 35;
const DESPAWN_DISTANCE = 200; // pixels behind player
const POPUP_DURATION = 1.5;

export interface NpcManagerState {
  /** Time until next NPC spawn */
  spawnTimer: number;
  /** Next NPC ID counter */
  nextId: number;
}

export function createNpcManagerState(): NpcManagerState {
  return {
    spawnTimer: randomInterval(),
    nextId: 1,
  };
}

function randomInterval(): number {
  return SPAWN_MIN_INTERVAL + Math.random() * (SPAWN_MAX_INTERVAL - SPAWN_MIN_INTERVAL);
}

/**
 * Update NPC positions and detect passing.
 * @param playerSpeed - Player speed in km/h
 * @returns Pop-up messages for passing events
 */
export function updateNpcs(
  npcs: NpcState[],
  manager: NpcManagerState,
  playerSpeed: number,
  dt: number,
): PopupMessage[] {
  const popups: PopupMessage[] = [];

  // Spawn timer
  manager.spawnTimer -= dt;
  if (manager.spawnTimer <= 0 && npcs.length < MAX_NPCS) {
    const speed = NPC_MIN_SPEED + Math.random() * (NPC_MAX_SPEED - NPC_MIN_SPEED);
    npcs.push({
      id: manager.nextId++,
      relativePos: 80 + Math.random() * 40, // spawn ahead
      speed,
      pedalAngle: Math.random() * Math.PI * 2,
      passed: false,
    });
    manager.spawnTimer = randomInterval();
  }

  // Update each NPC
  for (let i = npcs.length - 1; i >= 0; i--) {
    const npc = npcs[i];

    // Relative position: positive = ahead, negative = behind
    // Speed difference in km/h → pixels/sec (same scale as road)
    const speedDiff = npc.speed - playerSpeed;
    npc.relativePos += speedDiff * 2.0 * dt; // same SPEED_TO_PX factor

    // Pedal animation
    const cadenceEstimate = npc.speed * 2.5; // rough RPM estimate
    npc.pedalAngle = (npc.pedalAngle + (cadenceEstimate / 60) * Math.PI * 2 * dt) % (Math.PI * 2);

    // Passing detection
    if (!npc.passed && npc.relativePos < 0) {
      npc.passed = true;
      popups.push({
        text: 'PASSED!',
        color: '#44ff44',
        timeLeft: POPUP_DURATION,
        totalDuration: POPUP_DURATION,
      });
    }

    // Despawn if too far behind
    if (npc.relativePos < -DESPAWN_DISTANCE) {
      npcs.splice(i, 1);
    }
  }

  return popups;
}
