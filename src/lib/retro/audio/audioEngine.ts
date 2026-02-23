/**
 * Audio engine — lazy AudioContext with master gain and mute toggle.
 * Created on first un-mute to comply with browser autoplay policy.
 */

let audioCtx: AudioContext | null = null;
let masterGain: GainNode | null = null;
let muted = true;

/**
 * Get or create the AudioContext (lazy init).
 * Returns null if audio cannot be initialized.
 */
export function getAudioContext(): AudioContext | null {
  if (!audioCtx) {
    try {
      audioCtx = new AudioContext();
      masterGain = audioCtx.createGain();
      masterGain.gain.value = muted ? 0 : 0.3;
      masterGain.connect(audioCtx.destination);
    } catch {
      return null;
    }
  }
  // Resume if suspended (browser policy)
  if (audioCtx.state === 'suspended') {
    audioCtx.resume();
  }
  return audioCtx;
}

/**
 * Get the master gain node. Returns null if audio not initialized.
 */
export function getMasterGain(): GainNode | null {
  return masterGain;
}

/**
 * Set mute state. If un-muting for the first time, initializes AudioContext.
 */
export function setMuted(m: boolean): void {
  muted = m;
  if (!m) {
    // First un-mute triggers AudioContext creation
    getAudioContext();
  }
  if (masterGain) {
    masterGain.gain.value = m ? 0 : 0.3;
  }
}

export function isMuted(): boolean {
  return muted;
}
