/**
 * Sizes for the ride screen, computed from the stage (the space between the
 * header and the control bar). Pure so the AC12 minimums can be unit-tested;
 * RideStage measures itself and applies the result as CSS variables.
 *
 * Text widths assume IBM Plex Mono (0.6em per glyph). The widest primary value
 * is "1:02:09" (~4.3em incl. side bearings); a value with a unit like
 * "30.8 km/h" is narrower.
 */

export interface RideLayout {
  mode: 'wide' | 'stacked';
  powerPx: number;
  /** Side of the square the gauge is drawn in. */
  gaugePx: number;
  valuePx: number;
  smallValuePx: number;
  showSecondary: boolean;
  /** Height of the secondary strip in px (0 when hidden). */
  stripPx: number;
}

export const MIN_POWER_PX = 48;
export const MIN_VALUE_PX = 40;
export const MIN_SMALL_VALUE_PX = 28;

const GAP = 8; // grid gap between cards
const COL_GAP = 12; // between gauge and card columns
const CARD_PAD_X = 24; // horizontal padding inside a card (both sides)
const CARD_CHROME_Y = 50; // label + vertical padding + gap inside a card
const VALUE_EM = 4.3; // widest primary value, in em
const LINE = 1.15; // value line height
const WIDE_RATIO = 1.45;
const GAUGE_SHARE = 1 / 2.5; // gauge column is 1fr of "1fr 1.5fr"
const POWER_OF_GAUGE = 0.2; // power value size relative to the gauge's diameter

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const floor1 = (v: number) => Math.floor(v * 10) / 10;

function stripHeight(h: number): number {
  return Math.round(clamp(h * 0.11, 64, 96));
}

function smallValue(strip: number): number {
  return floor1(clamp((strip - 26) / LINE, MIN_SMALL_VALUE_PX, 48));
}

function fit(w: number, h: number, withStrip: boolean): RideLayout {
  const strip = withStrip ? stripHeight(h) : 0;
  const area = Math.max(0, h - (withStrip ? strip + GAP : 0)); // height above the strip
  const wide = h > 0 && w / h >= WIDE_RATIO;

  let valuePx: number;
  let gauge: number;
  if (wide) {
    const avail = Math.max(0, w - COL_GAP);
    const gaugeW = avail * GAUGE_SHARE;
    const cardW = (avail - gaugeW - GAP) / 2;
    const cardH = (area - GAP) / 2;
    valuePx = Math.min((cardW - CARD_PAD_X) / VALUE_EM, (cardH - CARD_CHROME_Y) / LINE);
    gauge = Math.min(gaugeW, area);
  } else {
    // Gauge on top, primaries 2x2 below.
    const gaugeH = area * 0.45;
    const cardW = (w - GAP) / 2;
    const cardH = (area - gaugeH - 2 * GAP) / 2;
    valuePx = Math.min((cardW - CARD_PAD_X) / VALUE_EM, (cardH - CARD_CHROME_Y) / LINE);
    gauge = Math.min(w, gaugeH);
  }

  return {
    mode: wide ? 'wide' : 'stacked',
    powerPx: floor1(Math.max(0, gauge * POWER_OF_GAUGE)),
    gaugePx: Math.floor(Math.max(0, gauge)),
    valuePx: floor1(Math.max(0, Math.min(valuePx, 200))),
    smallValuePx: withStrip ? smallValue(strip) : MIN_SMALL_VALUE_PX,
    showSecondary: withStrip,
    stripPx: strip,
  };
}

/**
 * Layout for a stage of w×h CSS px. The secondary strip is dropped first when
 * keeping it would push the primary numbers below their minimums; the ride
 * controls are outside the stage and never shrink.
 */
export function rideLayout(w: number, h: number): RideLayout {
  if (!(w > 0) || !(h > 0)) {
    return { mode: 'wide', powerPx: MIN_POWER_PX, gaugePx: MIN_POWER_PX / POWER_OF_GAUGE, valuePx: MIN_VALUE_PX, smallValuePx: MIN_SMALL_VALUE_PX, showSecondary: false, stripPx: 0 };
  }
  const withStrip = fit(w, h, true);
  if (withStrip.valuePx >= MIN_VALUE_PX && withStrip.powerPx >= MIN_POWER_PX) return withStrip;
  return fit(w, h, false);
}
