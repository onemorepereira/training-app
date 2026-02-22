export const metricTooltips: Record<string, string> = {
  // Analytics / PMC
  'Fitness': 'Chronic Training Load — a rolling average of your training stress over ~42 days',
  'Fatigue': 'Acute Training Load — a rolling average of your training stress over ~7 days',
  'Form': 'Training Stress Balance — the difference between fitness and fatigue (CTL − ATL)',
  'Week TSS': 'Total Training Stress Score accumulated this week',
  'Ramp': 'Rate of fitness change — how quickly your CTL is rising or falling per week',

  // Session — time
  'Duration': 'Total session duration',
  'Work': 'Total mechanical work performed',

  // Session — effort
  'NP': 'Normalized Power — a weighted average that accounts for the variability of your effort',
  'TSS': 'Training Stress Score — overall training load relative to your FTP',
  'IF': 'Intensity Factor — ratio of Normalized Power to your FTP',
  'VI': 'Variability Index — how uneven your power output was (NP / Avg Power)',

  // Session — power
  'Avg Power': 'Average power output across the session',
  'Max Power': 'Peak power recorded during the session',
  'FTP': 'Functional Threshold Power — the highest power you can sustain for approximately one hour',
  'PWC150': 'Physical Working Capacity at 150 bpm — estimated power at that heart rate',
  'PWC170': 'Physical Working Capacity at 170 bpm — estimated power at that heart rate',

  // Session — heart rate
  'Avg HR': 'Average heart rate during the session',
  'Max HR': 'Peak heart rate recorded during the session',

  // Session — movement
  'Avg Cadence': 'Average pedal cadence',
  'Avg Speed': 'Average speed during the session',
  'Distance': 'Total distance covered',
};
