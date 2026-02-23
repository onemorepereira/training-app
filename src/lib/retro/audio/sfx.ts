/**
 * SFX — oscillator-based 8-bit sound effects. No audio files needed.
 */

import { getAudioContext, getMasterGain, isMuted } from './audioEngine.js';

function playNote(
  freq: number,
  duration: number,
  type: OscillatorType = 'square',
  startDelay: number = 0,
  volume: number = 0.5,
): void {
  if (isMuted()) return;
  const ctx = getAudioContext();
  const master = getMasterGain();
  if (!ctx || !master) return;

  const osc = ctx.createOscillator();
  const gain = ctx.createGain();

  osc.type = type;
  osc.frequency.value = freq;
  gain.gain.value = volume;

  // Quick fade out to avoid clicks
  const startTime = ctx.currentTime + startDelay;
  const endTime = startTime + duration;
  gain.gain.setValueAtTime(volume, startTime);
  gain.gain.exponentialRampToValueAtTime(0.001, endTime);

  osc.connect(gain);
  gain.connect(master);

  osc.start(startTime);
  osc.stop(endTime + 0.01);
}

/** Zone change: square arpeggio (200ms) */
export function playZoneChange(zone: number): void {
  // Higher zones = higher pitch
  const baseFreq = 200 + zone * 60;
  playNote(baseFreq, 0.07, 'square', 0, 0.3);
  playNote(baseFreq * 1.25, 0.07, 'square', 0.07, 0.3);
  playNote(baseFreq * 1.5, 0.06, 'square', 0.14, 0.3);
}

/** PR jingle: 4-note major arpeggio (500ms) */
export function playPrJingle(): void {
  const notes = [523, 659, 784, 1047]; // C5-E5-G5-C6
  notes.forEach((freq, i) => {
    playNote(freq, 0.12, 'square', i * 0.12, 0.4);
  });
}

/** TSS milestone ding: triangle + noise (300ms) */
export function playMilestoneDing(): void {
  playNote(880, 0.15, 'triangle', 0, 0.5);
  playNote(1320, 0.15, 'triangle', 0.08, 0.3);

  // Noise burst
  if (isMuted()) return;
  const ctx = getAudioContext();
  const master = getMasterGain();
  if (!ctx || !master) return;

  const bufferSize = ctx.sampleRate * 0.05;
  const buffer = ctx.createBuffer(1, bufferSize, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < bufferSize; i++) {
    data[i] = (Math.random() * 2 - 1) * 0.1;
  }
  const noise = ctx.createBufferSource();
  noise.buffer = buffer;
  const gain = ctx.createGain();
  gain.gain.value = 0.15;
  gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.1);
  noise.connect(gain);
  gain.connect(master);
  noise.start();
}

/** NPC passed: 2-note chirp (100ms) */
export function playNpcPassed(): void {
  playNote(660, 0.05, 'square', 0, 0.3);
  playNote(880, 0.05, 'square', 0.05, 0.3);
}

/** Cadence tick: tiny noise burst (5ms) */
export function playCadenceTick(): void {
  if (isMuted()) return;
  const ctx = getAudioContext();
  const master = getMasterGain();
  if (!ctx || !master) return;

  const bufferSize = Math.round(ctx.sampleRate * 0.005);
  const buffer = ctx.createBuffer(1, bufferSize, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < bufferSize; i++) {
    data[i] = (Math.random() * 2 - 1) * 0.05;
  }
  const noise = ctx.createBufferSource();
  noise.buffer = buffer;
  const gain = ctx.createGain();
  gain.gain.value = 0.1;
  noise.connect(gain);
  gain.connect(master);
  noise.start();
}

/** Thunder: low noise sweep (800ms) */
export function playThunder(): void {
  if (isMuted()) return;
  const ctx = getAudioContext();
  const master = getMasterGain();
  if (!ctx || !master) return;

  const duration = 0.8;
  const bufferSize = Math.round(ctx.sampleRate * duration);
  const buffer = ctx.createBuffer(1, bufferSize, ctx.sampleRate);
  const data = buffer.getChannelData(0);
  for (let i = 0; i < bufferSize; i++) {
    const t = i / bufferSize;
    const envelope = Math.exp(-t * 4); // fast decay
    data[i] = (Math.random() * 2 - 1) * envelope * 0.3;
  }
  const noise = ctx.createBufferSource();
  noise.buffer = buffer;
  const gain = ctx.createGain();
  gain.gain.value = 0.4;
  noise.connect(gain);
  gain.connect(master);
  noise.start();
}
