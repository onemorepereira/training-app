/*
 * Tauri v2 IPC mock for rendering the training-app frontend in a plain browser.
 *
 * Load as an init script (Playwright `page.addInitScript({ path })`) BEFORE any
 * app code runs. It defines what @tauri-apps/api@2 reads:
 *   window.__TAURI_INTERNALS__ { invoke, transformCallback, unregisterCallback,
 *     runCallback, callbacks, convertFileSrc, metadata, plugins.path }
 *   window.__TAURI_EVENT_PLUGIN_INTERNALS__ { unregisterListener }
 * and answers every command in src/lib/tauri.ts plus plugin:event|*, plugin:app|*,
 * plugin:opener|*.
 *
 * State selection (first match wins):
 *   window.__MOCK_STATE (set by an earlier init script)
 *   ?mock=<state> query parameter
 *   localStorage['__MOCK_STATE']
 *   'normal'
 * States: 'normal' | 'empty' | 'error' | 'riding' | 'riding-zone'
 * Extra flags: ?prereq=fail (or window.__MOCK_PREREQ = 'fail') makes
 *   check_prerequisites report missing udev rules/BlueZ.
 *
 * Test hooks:
 *   window.__mockEmit(eventName, payload)  fire an event to listeners
 *   window.__mockCalls                     log of [cmd, args] invoked
 *   window.__mockSetState(state)           switch state at runtime (no reload)
 */
(function () {
  'use strict';

  const qs = new URLSearchParams(location.search);
  let STATE =
    window.__MOCK_STATE ||
    qs.get('mock') ||
    (function () { try { return localStorage.getItem('__MOCK_STATE'); } catch { return null; } })() ||
    'normal';
  const PREREQ_FAIL = (window.__MOCK_PREREQ || qs.get('prereq')) === 'fail';
  window.__MOCK_STATE = STATE;

  // ── Deterministic PRNG so screenshots are stable across runs ──────────
  function mulberry32(seed) {
    return function () {
      seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
      let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  const round = (v, d = 0) => { const f = 10 ** d; return Math.round(v * f) / f; };
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  // ── User config ───────────────────────────────────────────────────────
  const FTP = 250;
  const config = {
    ftp: FTP,
    weight_kg: 74.5,
    hr_zones: [124, 143, 162, 176, 190],
    units: 'metric',
    power_zones: [55, 75, 90, 105, 120, 150],
    date_of_birth: '1987-04-12',
    sex: 'male',
    resting_hr: 52,
    max_hr: 190,
  };
  const zonesSnapshot = { power_zones: config.power_zones.slice(), hr_zones: config.hr_zones.slice() };

  // ── Devices ───────────────────────────────────────────────────────────
  const isoAgo = (ms) => new Date(Date.now() - ms).toISOString();
  const devices = [
    { id: 'ble:C4:2F:90:11:7A:3E', name: 'HRM-Pro Plus', device_type: 'HeartRate', status: 'Connected', transport: 'Ble',
      rssi: -58, battery_level: 82, last_seen: isoAgo(2000), manufacturer: 'Garmin', model_number: 'HRM-Pro Plus', serial_number: '3392017745', device_group: null, in_range: true },
    { id: 'ant:11:54321', name: 'Assioma Duo', device_type: 'Power', status: 'Connected', transport: 'AntPlus',
      rssi: null, battery_level: 64, last_seen: isoAgo(1000), manufacturer: 'Favero', model_number: 'Assioma DUO', serial_number: '54321', device_group: null, in_range: true },
    { id: 'ble:E8:1D:A4:22:90:0B', name: 'KICKR CORE 5A1F', device_type: 'FitnessTrainer', status: 'Connected', transport: 'Ble',
      rssi: -49, battery_level: null, last_seen: isoAgo(1000), manufacturer: 'Wahoo Fitness', model_number: 'KICKR CORE', serial_number: '220510345', device_group: 'kickr-core-5a1f', in_range: true },
    { id: 'ant:17:23071', name: 'KICKR CORE 5A1F', device_type: 'FitnessTrainer', status: 'Disconnected', transport: 'AntPlus',
      rssi: null, battery_level: null, last_seen: isoAgo(86400000), manufacturer: 'Wahoo Fitness', model_number: 'KICKR CORE', serial_number: '23071', device_group: 'kickr-core-5a1f', in_range: true },
    { id: 'ble:D0:7B:33:5C:18:F2', name: 'Wahoo CADENCE 1C2B', device_type: 'CadenceSpeed', status: 'Disconnected', transport: 'Ble',
      rssi: null, battery_level: 41, last_seen: isoAgo(3 * 86400000), manufacturer: 'Wahoo Fitness', model_number: 'CADENCE', serial_number: '1C2B9940', device_group: null, in_range: false },
  ];
  const primaries = { HeartRate: devices[0].id, Power: devices[1].id, FitnessTrainer: devices[2].id };

  function deviceDetails(id) {
    const d = devices.find((x) => x.id === id);
    if (!d) throw err('device_not_found', `Device not found: ${id}`);
    const svc = (uuid16, name, chars) => ({
      uuid: `0000${uuid16}-0000-1000-8000-00805f9b34fb`, name,
      characteristics: chars.map(([u, n, p]) => ({ uuid: `0000${u}-0000-1000-8000-00805f9b34fb`, name: n, properties: p })),
    });
    const services = d.transport === 'AntPlus' ? [] : [
      svc('1800', 'Generic Access', [['2a00', 'Device Name', ['Read']], ['2a01', 'Appearance', ['Read']]]),
      svc('180a', 'Device Information', [['2a29', 'Manufacturer Name String', ['Read']], ['2a24', 'Model Number String', ['Read']], ['2a25', 'Serial Number String', ['Read']], ['2a26', 'Firmware Revision String', ['Read']]]),
      svc('180f', 'Battery Service', [['2a19', 'Battery Level', ['Read', 'Notify']]]),
      ...(d.device_type === 'HeartRate' ? [svc('180d', 'Heart Rate', [['2a37', 'Heart Rate Measurement', ['Notify']], ['2a38', 'Body Sensor Location', ['Read']]])] : []),
      ...(d.device_type === 'FitnessTrainer' ? [
        svc('1826', 'Fitness Machine', [['2acc', 'Fitness Machine Feature', ['Read']], ['2ad2', 'Indoor Bike Data', ['Notify']], ['2ad9', 'Fitness Machine Control Point', ['Write', 'Indicate']], ['2ada', 'Fitness Machine Status', ['Notify']]]),
        svc('1818', 'Cycling Power', [['2a63', 'Cycling Power Measurement', ['Notify']], ['2a65', 'Cycling Power Feature', ['Read']]]),
        { uuid: 'a026ee0b-0a7d-4ab3-97fa-f1500f9feb8b', name: null, characteristics: [{ uuid: 'a026e037-0a7d-4ab3-97fa-f1500f9feb8b', name: null, properties: ['Write', 'Notify'] }] },
      ] : []),
      ...(d.device_type === 'CadenceSpeed' ? [svc('1816', 'Cycling Speed and Cadence', [['2a5b', 'CSC Measurement', ['Notify']], ['2a5c', 'CSC Feature', ['Read']]])] : []),
    ];
    return {
      id: d.id, name: d.name, device_type: d.device_type, transport: d.transport, rssi: d.rssi,
      battery_level: d.battery_level, manufacturer: d.manufacturer, model_number: d.model_number,
      serial_number: d.serial_number, firmware_revision: d.transport === 'Ble' ? '4.2.1' : null,
      hardware_revision: d.transport === 'Ble' ? 'B' : null, software_revision: null, services,
    };
  }

  // ── Sessions ──────────────────────────────────────────────────────────
  const ACTIVITY_TYPES = ['endurance', 'intervals', 'threshold', 'sweet_spot', 'vo2max', 'tempo', 'recovery'];
  const sessions = [];
  const zoneRideConfigs = {};
  (function buildSessions() {
    const rnd = mulberry32(1337);
    const now = new Date();
    const N = 25;
    for (let i = 0; i < N; i++) {
      // Spread over the last 56 days, newest first, typical evening/morning slots
      const daysAgo = Math.round((i / N) * 55 + rnd() * 1.5);
      const start = new Date(now);
      start.setDate(start.getDate() - daysAgo);
      const hour = [6, 7, 12, 17, 18, 19][Math.floor(rnd() * 6)];
      start.setHours(hour, Math.floor(rnd() * 60), Math.floor(rnd() * 60), 0);
      if (start > now) start.setDate(start.getDate() - 1);
      const kind = ACTIVITY_TYPES[Math.floor(rnd() * ACTIVITY_TYPES.length)];
      const duration = Math.round((kind === 'recovery' ? 1800 : kind === 'endurance' ? 5400 + rnd() * 3600 : 2700 + rnd() * 2700));
      const ifTarget = { endurance: 0.66, intervals: 0.86, threshold: 0.92, sweet_spot: 0.86, vo2max: 0.88, tempo: 0.8, recovery: 0.52 }[kind] + (rnd() - 0.5) * 0.06;
      // FTP progression: 235 → 250 over 8 weeks
      const ftp = i > 16 ? 235 : i > 8 ? 242 : 250;
      const np = Math.round(ftp * ifTarget);
      const vi = kind === 'endurance' || kind === 'recovery' ? 1.02 + rnd() * 0.02 : 1.06 + rnd() * 0.08;
      const avgPower = Math.round(np / vi);
      const IF = round(np / ftp, 2);
      const tss = round((duration * np * IF) / (ftp * 3600) * 100, 1);
      const avgHr = Math.round(108 + IF * 52 + rnd() * 6);
      const s = {
        id: `s-${String(1000 + N - i)}-${start.toISOString().slice(0, 10)}`,
        start_time: start.toISOString(),
        duration_secs: duration,
        ftp,
        avg_power: avgPower,
        max_power: Math.round(np * (kind === 'vo2max' || kind === 'intervals' ? 2.6 : 1.9) + rnd() * 120),
        normalized_power: np,
        tss,
        intensity_factor: IF,
        avg_hr: i === 13 ? null : avgHr, // one ride without HR strap
        max_hr: i === 13 ? null : Math.min(190, Math.round(avgHr + 18 + rnd() * 14)),
        avg_cadence: round(84 + rnd() * 10, 1),
        avg_speed: round(28 + (avgPower - 150) * 0.07 + rnd() * 2, 1),
        work_kj: round((avgPower * duration) / 1000, 1),
        variability_index: round(vi, 2),
        distance_km: null,
        zones: i < 20 ? { power_zones: zonesSnapshot.power_zones.slice(), hr_zones: zonesSnapshot.hr_zones.slice() } : null,
      };
      s.distance_km = round((s.avg_speed * duration) / 3600, 1);
      if (i % 4 === 1) { s.activity_type = kind; }
      if (i % 6 === 2) { s.title = { 0: 'Morning spin', 1: 'Lunch intervals', 2: 'Over-unders', 3: 'Long Z2', 4: 'FTP builder', 5: 'Sweet spot 3x15' }[i % 6] || `${kind} session`; }
      if (i % 5 === 3) { s.rpe = 4 + Math.floor(rnd() * 5); }
      sessions.push(s);
    }
    // The showcase session: notes, activity type, RPE, title and a zone ride.
    const star = sessions[1];
    star.id = 'demo-threshold-2x20'; // stable id for /history/<id> screenshots
    star.title = 'Threshold 2x20 on the KICKR';
    star.activity_type = 'threshold';
    star.rpe = 8;
    star.notes = 'Legs felt heavy for the first 10 min, then settled. Second interval held 255 W with HR drifting to 172. Fan on max. Next time: open at 245 and build.';
    Object.assign(star, {
      duration_secs: 3900, ftp: FTP, normalized_power: 236, avg_power: 221, max_power: 612,
      intensity_factor: 0.94, variability_index: 1.07, avg_hr: 158, max_hr: 176,
      tss: round((3900 * 236 * (236 / FTP)) / (FTP * 3600) * 100, 1),
      work_kj: round((221 * 3900) / 1000, 1), avg_speed: 34.6, distance_km: round((34.6 * 3900) / 3600, 1),
    });
    // Zone-ride sessions
    for (const idx of [1, 4, 9, 15]) {
      const s = sessions[idx];
      const mode = idx === 9 ? 'HeartRate' : 'Power';
      const zone = mode === 'HeartRate' ? 2 : idx === 1 ? 4 : 3;
      const lb = mode === 'HeartRate' ? config.hr_zones[zone - 2] : Math.round((config.power_zones[zone - 2] / 100) * FTP);
      const ub = mode === 'HeartRate' ? config.hr_zones[zone - 1] : Math.round((config.power_zones[zone - 1] / 100) * FTP);
      const dur = Math.min(2400, s.duration_secs - 600);
      const series = [];
      let cmd = mode === 'HeartRate' ? 150 : lb - 20;
      for (let t = 0; t < dur; t += 5) {
        const target = mode === 'HeartRate' ? 168 + Math.sin(t / 300) * 6 : (lb + ub) / 2;
        cmd += (target - cmd) * 0.15;
        series.push(Math.round(cmd));
      }
      zoneRideConfigs[s.id] = JSON.stringify({
        mode, zone, lower_bound: lb, upper_bound: ub, duration_secs: dur,
        time_in_zone_secs: Math.round(dur * (mode === 'HeartRate' ? 0.78 : 0.88)),
        commanded_power_series: series,
        time_to_zone_secs: mode === 'HeartRate' ? 214 : 38,
      });
    }
  })();

  // ── Session analysis (full 1 Hz timeseries) ───────────────────────────
  const POWER_CURVE_DURATIONS = [1, 2, 3, 5, 10, 15, 20, 30, 45, 60, 120, 300, 600, 1200, 1800, 3600];
  function powerProfileAt(t, dur, s) {
    // Warmup 10 min, structured main set, cooldown 8 min.
    const np = s.normalized_power || 180;
    const warm = 600, cool = 480;
    if (t < warm) return 110 + (t / warm) * (np * 0.75 - 110);
    if (t > dur - cool) return np * 0.55;
    const k = s.activity_type || (s.intensity_factor > 0.84 ? 'intervals' : 'endurance');
    const mt = t - warm;
    if (k === 'threshold') { const blk = mt % 1500; return blk < 1200 ? FTP * 1.0 : FTP * 0.55; }
    if (k === 'intervals' || k === 'vo2max') { const blk = mt % 480; return blk < 240 ? FTP * 1.15 : FTP * 0.5; }
    if (k === 'sweet_spot') { const blk = mt % 1200; return blk < 900 ? FTP * 0.9 : FTP * 0.55; }
    return np * 0.98;
  }
  function analysisFor(id) {
    const s = sessions.find((x) => x.id === id);
    if (!s) throw err('session_error', `Session error: session ${id} not found`);
    const rnd = mulberry32(id.split('').reduce((a, c) => a + c.charCodeAt(0), 0));
    const dur = s.duration_secs;
    const ts = [];
    let hr = 95, sprintAt = Math.floor(dur * 0.62);
    const hasHr = s.avg_hr != null;
    const pauseStart = Math.floor(dur * 0.45), pauseEnd = pauseStart + 45; // short pause gap
    for (let t = 0; t < dur; t++) {
      if (t >= pauseStart && t < pauseEnd) {
        ts.push({ elapsed_secs: t, power: null, heart_rate: null, cadence: null, speed: null });
        continue;
      }
      let p = powerProfileAt(t, dur, s) * (1 + (rnd() - 0.5) * 0.16);
      if (t >= sprintAt && t < sprintAt + 12) p = (s.max_power || 700) * (0.85 + rnd() * 0.15);
      p = Math.max(0, Math.round(p));
      const hrTarget = 70 + (p / FTP) * 95;
      hr += (clamp(hrTarget, 90, 186) - hr) * 0.02;
      const cad = p < 20 ? 0 : Math.round(80 + (p / FTP) * 12 + (rnd() - 0.5) * 6);
      const spd = round(18 + Math.cbrt(p) * 2.3 + (rnd() - 0.5), 1);
      ts.push({ elapsed_secs: t, power: p, heart_rate: hasHr ? Math.round(hr + (rnd() - 0.5) * 2) : null, cadence: cad, speed: spd });
    }
    // Power curve (max mean power)
    const pw = ts.map((x) => x.power ?? 0);
    const pre = new Float64Array(pw.length + 1);
    for (let i = 0; i < pw.length; i++) pre[i + 1] = pre[i] + pw[i];
    const power_curve = POWER_CURVE_DURATIONS.filter((d) => d <= pw.length).map((d) => {
      let best = 0;
      for (let i = d; i <= pw.length; i++) best = Math.max(best, pre[i] - pre[i - d]);
      return { duration_secs: d, watts: Math.round(best / d) };
    });
    // Zone distributions
    const pBounds = config.power_zones.map((p) => (p / 100) * (s.ftp || FTP));
    const pz = new Array(7).fill(0), hz = new Array(5).fill(0);
    for (const x of ts) {
      if (x.power != null) { let z = pBounds.findIndex((b) => x.power <= b); pz[z === -1 ? 6 : z]++; }
      if (x.heart_rate != null) { let z = config.hr_zones.findIndex((b) => x.heart_rate <= b); hz[z === -1 ? 4 : z]++; }
    }
    const toBuckets = (arr) => { const tot = arr.reduce((a, b) => a + b, 0); return arr.map((v, i) => ({ zone: i + 1, duration_secs: v, percentage: tot ? (v / tot) * 100 : 0 })); };
    return {
      timeseries: ts,
      power_curve,
      power_zone_distribution: toBuckets(pz),
      hr_zone_distribution: hasHr ? toBuckets(hz) : [],
      pwc: hasHr ? { pwc150: 168.4, pwc170: 231.9, r_squared: 0.87, sample_count: 1812 } : null,
      zones: s.zones || zonesSnapshot,
    };
  }
  function bestCurve(period) {
    const scale = { '7d': 0.96, '30d': 0.99, '90d': 1.0, '365d': 1.0, all: 1.0 }[period] ?? 1.0;
    const base = [[1, 1012], [2, 998], [3, 971], [5, 905], [10, 802], [15, 702], [20, 640], [30, 552], [45, 455], [60, 402], [120, 338], [300, 296], [600, 274], [1200, 262], [1800, 252], [3600, 241]];
    return base.map(([d, w]) => ({ duration_secs: d, watts: Math.round(w * scale) }));
  }

  // ── Live ride simulation ──────────────────────────────────────────────
  const ride = {
    active: STATE === 'riding' || STATE === 'riding-zone',
    paused: false,
    id: 's-live-' + new Date().toISOString().slice(0, 10),
    elapsed: 1834,
    powers: [],
    timer: null,
    zone: STATE === 'riding-zone'
      ? { mode: 'Power', zone: 3, lower: 188, upper: 225, start: 1534, duration: 1200, inZone: 241 }
      : null,
  };
  function seedPowerHistory() {
    const r = mulberry32(99);
    for (let i = 0; i < ride.elapsed; i++) ride.powers.push(Math.round(200 + Math.sin(i / 40) * 25 + (r() - 0.5) * 30));
  }
  function liveTick() {
    if (!ride.active || ride.paused) return;
    ride.elapsed += 1;
    const t = ride.elapsed;
    const base = ride.zone ? (ride.zone.lower + ride.zone.upper) / 2 : 205;
    const power = Math.max(0, Math.round(base + Math.sin(t / 9) * 22 + (Math.random() - 0.5) * 24));
    ride.powers.push(power);
    if (ride.zone && power >= ride.zone.lower && power <= ride.zone.upper) ride.zone.inZone += 1;
    const avg = (n) => { const a = ride.powers.slice(-n); return a.reduce((x, y) => x + y, 0) / a.length; };
    // NP over whole ride (30 s rolling, ^4)
    let acc = 0, cnt = 0;
    for (let i = 29; i < ride.powers.length; i += 5) {
      let s = 0; for (let j = i - 29; j <= i; j++) s += ride.powers[j];
      acc += (s / 30) ** 4; cnt++;
    }
    const np = cnt ? Math.pow(acc / cnt, 0.25) : null;
    const IF = np ? np / FTP : null;
    const tss = np ? (t * np * IF) / (FTP * 3600) * 100 : null;
    const hr = Math.round(148 + Math.sin(t / 60) * 6 + (power - 205) * 0.08);
    const cadence = round(89 + Math.sin(t / 13) * 4 + (Math.random() - 0.5) * 2, 1);
    const speed = round(32.4 + Math.sin(t / 30) * 1.6, 1);
    const pZone = (() => { const z = config.power_zones.findIndex((p) => power <= (p / 100) * FTP); return z === -1 ? 7 : z + 1; })();
    const hZone = (() => { const z = config.hr_zones.findIndex((b) => hr <= b); return z === -1 ? 5 : z + 1; })();
    const now = Date.now();
    emit('sensor_reading', { Power: { watts: power, epoch_ms: now, device_id: devices[1].id, pedal_balance: 51 } });
    emit('sensor_reading', { HeartRate: { bpm: hr, epoch_ms: now, device_id: devices[0].id } });
    emit('sensor_reading', { Cadence: { rpm: cadence, epoch_ms: now, device_id: devices[2].id } });
    emit('sensor_reading', { Speed: { kmh: speed, epoch_ms: now, device_id: devices[2].id } });
    emit('live_metrics', {
      elapsed_secs: t,
      current_power: power,
      avg_power_3s: round(avg(3), 1),
      avg_power_10s: round(avg(10), 1),
      avg_power_30s: round(avg(30), 1),
      normalized_power: np != null ? round(np, 1) : null,
      tss: tss != null ? round(tss, 1) : null,
      intensity_factor: IF != null ? round(IF, 3) : null,
      current_hr: hr,
      current_cadence: cadence,
      current_speed: speed,
      hr_zone: hZone,
      power_zone: pZone,
      stale_power: false,
      stale_hr: false,
      stale_cadence: false,
      stale_speed: false,
    });
  }
  function startLive() {
    if (ride.timer) return;
    if (!ride.powers.length) seedPowerHistory();
    ride.timer = setInterval(liveTick, 1000);
    // First tick soon after listeners register so the first screenshot has data.
    setTimeout(liveTick, 300);
  }
  function stopLive() { if (ride.timer) clearInterval(ride.timer); ride.timer = null; }
  function zoneStatus() {
    const z = ride.zone;
    if (!z || !ride.active) {
      return { active: false, mode: null, target_zone: null, lower_bound: null, upper_bound: null, commanded_power: null,
        time_in_zone_secs: 0, elapsed_secs: 0, duration_secs: null, paused: false, phase: 'idle', safety_note: null, stop_reason: null };
    }
    const el = ride.elapsed - z.start;
    return { active: true, mode: z.mode, target_zone: z.zone, lower_bound: z.lower, upper_bound: z.upper,
      commanded_power: Math.round((z.lower + z.upper) / 2 + Math.sin(el / 20) * 6), time_in_zone_secs: z.inZone,
      elapsed_secs: el, duration_secs: z.duration, paused: ride.paused, phase: ride.paused ? 'paused' : 'in_zone', safety_note: null, stop_reason: null };
  }

  // ── IPC plumbing ──────────────────────────────────────────────────────
  const callbacks = new Map();
  let nextCb = 1;
  function transformCallback(cb, once = false) {
    const id = nextCb++;
    callbacks.set(id, (data) => { if (once) callbacks.delete(id); return cb && cb(data); });
    return id;
  }
  function unregisterCallback(id) { callbacks.delete(id); }
  function runCallback(id, data) { const f = callbacks.get(id); if (f) f(data); }

  const listeners = new Map(); // event -> [{id, handler}]
  let nextEventId = 1;
  function emit(event, payload) {
    const ls = listeners.get(event) || [];
    for (const l of ls.slice()) runCallback(l.handler, { event, id: l.id, payload });
  }

  function err(code, message) { return { code, message }; }
  const ERRORS = {
    devices: err('ble_error', 'BLE error: No Bluetooth adapter found (org.bluez.Error.NotReady)'),
    sessions: err('database_error', 'Database error: error returned from database: (code: 5) database is locked'),
    default: err('session_error', 'Session error: backend unavailable'),
  };
  const DEVICE_CMDS = new Set(['get_known_devices', 'scan_devices', 'connect_device', 'get_device_details', 'disconnect_device',
    'set_primary_device', 'get_primary_devices', 'unlink_devices', 'set_trainer_power', 'set_trainer_resistance',
    'set_trainer_simulation', 'start_trainer', 'stop_trainer', 'check_prerequisites', 'fix_prerequisites']);
  const clone = (v) => (v === undefined ? v : JSON.parse(JSON.stringify(v)));
  const calls = [];
  window.__mockCalls = calls;

  const empty = () => STATE === 'empty';

  const handlers = {
    // devices
    get_known_devices: () => (empty() ? [] : devices),
    scan_devices: () => (empty() ? [] : devices.map((d) => ({ ...d, in_range: true }))),
    connect_device: ({ deviceId }) => {
      const d = devices.find((x) => x.id === deviceId);
      if (!d) throw err('device_not_found', `Device not found: ${deviceId}`);
      d.status = 'Connected'; d.in_range = true; d.last_seen = new Date().toISOString();
      return d;
    },
    disconnect_device: ({ deviceId }) => { const d = devices.find((x) => x.id === deviceId); if (d) d.status = 'Disconnected'; return null; },
    get_device_details: ({ deviceId }) => deviceDetails(deviceId),
    set_primary_device: ({ deviceType, deviceId }) => { primaries[deviceType] = deviceId; return null; },
    get_primary_devices: () => (empty() ? {} : primaries),
    unlink_devices: ({ deviceId }) => { for (const d of devices) if (d.device_group && devices.find((x) => x.id === deviceId)?.device_group === d.device_group) d.device_group = null; return null; },
    // session lifecycle
    start_session: () => { ride.active = true; ride.paused = false; ride.elapsed = 0; ride.powers = [0]; startLive(); return ride.id; },
    stop_session: () => {
      if (!ride.active) return null;
      ride.active = false; stopLive();
      const ps = ride.powers.filter((p) => p > 0);
      const avgP = ps.length ? Math.round(ps.reduce((a, b) => a + b, 0) / ps.length) : null;
      return { id: ride.id, start_time: new Date(Date.now() - ride.elapsed * 1000).toISOString(), duration_secs: ride.elapsed, ftp: FTP,
        avg_power: avgP, max_power: ps.length ? Math.max(...ps) : null, normalized_power: avgP ? avgP + 6 : null,
        tss: avgP ? round((ride.elapsed * (avgP + 6) * ((avgP + 6) / FTP)) / (FTP * 3600) * 100, 1) : null,
        intensity_factor: avgP ? round((avgP + 6) / FTP, 2) : null, avg_hr: 149, max_hr: 168, avg_cadence: 89.2, avg_speed: 32.1,
        work_kj: avgP ? round((avgP * ride.elapsed) / 1000, 1) : null, variability_index: 1.03, distance_km: round((32.1 * ride.elapsed) / 3600, 1), zones: zonesSnapshot };
    },
    get_session_status: () => (ride.active ? { session_id: ride.id, paused: ride.paused } : null),
    pause_session: () => { ride.paused = true; return null; },
    resume_session: () => { ride.paused = false; return null; },
    list_sessions: () => (empty() ? [] : sessions),
    get_session: ({ sessionId }) => {
      const s = empty() ? null : sessions.find((x) => x.id === sessionId);
      if (!s) throw err('session_error', `Session error: session ${sessionId} not found`);
      return s;
    },
    get_session_analysis: ({ sessionId }) => { if (empty()) throw err('session_error', `Session error: session ${sessionId} not found`); return analysisFor(sessionId); },
    get_user_config: () => config,
    save_user_config: ({ config: c }) => { Object.assign(config, c); return null; },
    update_session_metadata: ({ sessionId, title, activityType, rpe, notes }) => {
      const s = sessions.find((x) => x.id === sessionId);
      if (s) { s.title = title ?? undefined; s.activity_type = activityType ?? undefined; s.rpe = rpe ?? undefined; s.notes = notes ?? undefined; }
      return null;
    },
    delete_session: ({ sessionId }) => { const i = sessions.findIndex((x) => x.id === sessionId); if (i >= 0) sessions.splice(i, 1); return null; },
    export_session_fit: ({ sessionId }) => `/home/rider/Downloads/${sessionId}.fit`,
    // trainer
    set_trainer_power: () => null, set_trainer_resistance: () => null, set_trainer_simulation: () => null,
    start_trainer: () => null, stop_trainer: () => null,
    // zone control
    start_zone_control: ({ target }) => {
      ride.zone = { mode: target.mode, zone: target.zone, lower: target.lower_bound, upper: target.upper_bound, start: ride.elapsed, duration: target.duration_secs, inZone: 0 };
      return null;
    },
    stop_zone_control: () => { const was = !!ride.zone; ride.zone = null; return was ? 'UserStopped' : null; },
    pause_zone_control: () => null, resume_zone_control: () => null,
    get_zone_control_status: () => zoneStatus(),
    estimate_initial_power: ({ targetHr }) => Math.round(80 + (targetHr - 100) * 1.9),
    save_zone_ride_config: ({ sessionId, zoneConfig }) => { zoneRideConfigs[sessionId] = zoneConfig; return null; },
    get_zone_ride_config: ({ sessionId }) => zoneRideConfigs[sessionId] ?? null,
    get_best_power_curve: ({ period }) => (empty() ? [] : bestCurve(period)),
    backfill_power_curves: () => 0,
    // prerequisites
    check_prerequisites: () => PREREQ_FAIL
      ? { udev_rules: false, bluez_installed: true, bluetooth_service: false, all_met: false, pkexec_available: true }
      : { udev_rules: true, bluez_installed: true, bluetooth_service: true, all_met: true, pkexec_available: true },
    fix_prerequisites: () => ({ success: true, message: 'Installed udev rules and started bluetooth.service',
      status: { udev_rules: true, bluez_installed: true, bluetooth_service: true, all_met: true, pkexec_available: true } }),
    // dev simulator
    sim_start: () => null, sim_stop: () => null,
    sim_status: () => ({ status: ride.active ? 'Running' : 'Stopped', profile: 'SteadyState' }),
    // core plugins
    'plugin:app|version': () => '0.1.0',
    'plugin:app|name': () => 'Training App',
    'plugin:app|tauri_version': () => '2.10.1',
    'plugin:app|identifier': () => 'com.training-app',
    'plugin:opener|open_url': ({ url }) => { console.info('[tauri-mock] open_url', url); return null; },
    'plugin:opener|open_path': () => null,
    'plugin:event|listen': ({ event, handler }) => {
      const id = nextEventId++;
      if (!listeners.has(event)) listeners.set(event, []);
      listeners.get(event).push({ id, handler });
      return id;
    },
    'plugin:event|unlisten': ({ event, eventId }) => {
      const ls = listeners.get(event);
      if (ls) listeners.set(event, ls.filter((l) => l.id !== eventId));
      return null;
    },
    'plugin:event|emit': ({ event, payload }) => { emit(event, payload); return null; },
    'plugin:event|emit_to': ({ event, payload }) => { emit(event, payload); return null; },
  };

  const NEVER_FAIL = (cmd) => cmd.startsWith('plugin:event|') || cmd.startsWith('plugin:app|') || cmd.startsWith('plugin:opener|');

  async function invoke(cmd, args = {}, _options) {
    calls.push([cmd, args]);
    // Small async delay like a real IPC round-trip.
    await new Promise((r) => setTimeout(r, cmd.startsWith('plugin:') ? 0 : 15));
    if (STATE === 'error' && !NEVER_FAIL(cmd)) {
      // sim_status rejecting hides the dev toolbar (as in a production build).
      throw clone(DEVICE_CMDS.has(cmd) ? ERRORS.devices : cmd.includes('session') || cmd.includes('power_curve') ? ERRORS.sessions : ERRORS.default);
    }
    const h = handlers[cmd];
    if (!h) {
      console.warn('[tauri-mock] unhandled command', cmd, args);
      throw `Command ${cmd} not found`;
    }
    return clone(h(args || {}));
  }

  window.__TAURI_INTERNALS__ = Object.assign(window.__TAURI_INTERNALS__ || {}, {
    invoke,
    transformCallback,
    unregisterCallback,
    runCallback,
    callbacks,
    convertFileSrc: (p, protocol = 'asset') => `${protocol}://localhost/${encodeURIComponent(p)}`,
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    plugins: { path: { sep: '/', delimiter: ':' } },
  });
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = Object.assign(window.__TAURI_EVENT_PLUGIN_INTERNALS__ || {}, {
    unregisterListener: (event, eventId) => {
      const ls = listeners.get(event);
      if (ls) listeners.set(event, ls.filter((l) => l.id !== eventId));
    },
  });
  window.__mockEmit = emit;
  window.__mockSessions = sessions;
  window.__mockSetState = (s) => { STATE = s; window.__MOCK_STATE = s; };

  if (ride.active) startLive();
})();
