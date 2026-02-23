/**
 * SNES-style color palettes keyed by power zone.
 * Each palette provides colors for the cyclist sprite.
 *
 * Key design: bike frame is always RED across all zones (like a real bike).
 * Only the jersey color changes per zone.
 */

export interface ZonePalette {
  jersey: string;
  jerseyDark: string;
  skin: string;
  skinDark: string;
  /** Bike frame — bright red, constant across zones */
  bike: string;
  /** Bike frame shadow — dark red */
  bikeDark: string;
  /** Wheel rim — silver */
  wheel: string;
  /** Accent (helmet) — white */
  accent: string;
  hair: string;
  /** Tire / outline — dark */
  outline: string;
}

// Shared bike/body colors (constant across all zones)
const BIKE = '#cc3333';
const BIKE_DARK = '#992222';
const WHEEL = '#aabbcc';
const SKIN = '#f0d0a8';
const SKIN_DARK = '#d4b088';
const HAIR = '#775533';
const ACCENT = '#eeeeff';  // white helmet
const OUTLINE = '#444455';  // tire color

/** Zone palettes: index 0 = idle/Z0, 1-7 = power zones */
export const ZONE_PALETTES: ZonePalette[] = [
  // 0: Idle — muted grey jersey
  { jersey: '#8899aa', jerseyDark: '#6a7b8c', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 1: Recovery — light blue
  { jersey: '#55bbee', jerseyDark: '#3399cc', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 2: Endurance — green
  { jersey: '#44cc66', jerseyDark: '#33aa44', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 3: Tempo — yellow
  { jersey: '#eecc22', jerseyDark: '#ccaa00', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 4: Threshold — orange
  { jersey: '#ff9933', jerseyDark: '#dd7711', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 5: VO2max — red
  { jersey: '#ff4444', jerseyDark: '#dd2222', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 6: Anaerobic — magenta
  { jersey: '#ee44bb', jerseyDark: '#cc2299', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
  // 7: Neuromuscular — purple
  { jersey: '#cc55ff', jerseyDark: '#aa33dd', skin: SKIN, skinDark: SKIN_DARK,
    bike: BIKE, bikeDark: BIKE_DARK, wheel: WHEEL, accent: ACCENT, hair: HAIR, outline: OUTLINE },
];

/** Road and environment colors */
export const ENV_COLORS = {
  road: '#555564',
  roadNear: '#606070',
  roadLine: '#b0b098',
  shoulder: '#707068',
  grassNear: '#3a7a3a',
  grassFar: '#2d5e2d',
  grassHighlight: '#4a8f4a',
  skyTop: '#3355aa',
  skyHorizon: '#99bbdd',
} as const;

/** HUD colors per zone for the zone meter */
export const ZONE_HUD_COLORS = [
  '#8899aa', // 0: idle
  '#55bbee', // 1: recovery
  '#44cc66', // 2: endurance
  '#eecc22', // 3: tempo
  '#ff9933', // 4: threshold
  '#ff4444', // 5: VO2max
  '#ee44bb', // 6: anaerobic
  '#cc55ff', // 7: neuromuscular
];
