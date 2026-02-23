/** Effort levels mapped from power zones */
export type EffortLevel = 'idle' | 'easy' | 'tempo' | 'threshold' | 'vo2' | 'anaerobic' | 'sprint';

/** Effort levels ordered by intensity for indexing */
export const EFFORT_LEVELS: EffortLevel[] = [
  'idle', 'easy', 'tempo', 'threshold', 'vo2', 'anaerobic', 'sprint'
];

/** Map power zone (1-7) to effort level. Zone 0 or null = idle */
export function zoneToEffort(zone: number | null): EffortLevel {
  if (zone == null || zone < 1 || zone > 7) return 'idle';
  return EFFORT_LEVELS[zone - 1] ?? 'idle';
}

/** Current cyclist visual state */
export interface CyclistState {
  effort: EffortLevel;
  /** Pedal rotation angle in radians [0, 2*PI) */
  pedalAngle: number;
  /** Vertical bob offset in pixels (synced to pedal) */
  bobY: number;
}

/** Sensor snapshot read each frame */
export interface SensorSnapshot {
  power: number | null;
  hr: number | null;
  cadence: number | null;
  speed: number | null;
  powerZone: number | null;
  elapsedSecs: number;
  tss: number | null;
}

/** A queued pop-up message */
export interface PopupMessage {
  text: string;
  color: string;
  /** Time remaining in seconds */
  timeLeft: number;
  /** Total duration for animation progress */
  totalDuration: number;
}

/** NPC cyclist state */
export interface NpcState {
  id: number;
  /** Relative road position (0 = same as player, positive = ahead) */
  relativePos: number;
  /** NPC speed in km/h */
  speed: number;
  /** Pedal angle for animation */
  pedalAngle: number;
  /** Whether player has passed this NPC */
  passed: boolean;
}

/** Zone streak tracking */
export interface ZoneStreak {
  zone: number;
  /** Duration in seconds */
  duration: number;
}

/** Central game state */
export interface GameState {
  cyclist: CyclistState;
  /** Road scroll offset in pixels */
  roadOffset: number;
  /** Current sensor snapshot */
  sensors: SensorSnapshot;
  /** Time-of-day factor 0-1 (0=midnight, 0.25=sunrise, 0.5=noon, 0.75=sunset) */
  timeOfDay: number;
  /** Active pop-up messages */
  popups: PopupMessage[];
  /** Active NPCs */
  npcs: NpcState[];
  /** Current zone streak */
  streak: ZoneStreak | null;
  /** Cumulative score */
  score: number;
  /** Current combo multiplier */
  comboMultiplier: number;
  /** Whether audio is muted */
  muted: boolean;
  /** Frame count for animation timing */
  frameCount: number;
  /** Climb grade 0.0 (flat) to 1.0 (steep), derived from power zone + cadence */
  climbGrade: number;
}

/** Internal canvas resolution */
export const CANVAS_WIDTH = 384;
export const CANVAS_HEIGHT = 216;
