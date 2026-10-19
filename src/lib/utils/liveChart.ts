export interface ZoneBand {
  lower: number;
  upper: number;
}

type MarkAreaPoint = { yAxis: number; itemStyle?: { color: string } };

/**
 * markArea option for the live power series. Always returns an object (with
 * empty `data` when there is no band) because ECharts' default merge mode keeps
 * a previous markArea when the key is omitted; an explicit array replaces it.
 */
export function zoneMarkArea(band: ZoneBand | null): {
  silent: true;
  data: [MarkAreaPoint, MarkAreaPoint][];
} {
  if (!band) return { silent: true, data: [] };
  return {
    silent: true,
    data: [[
      { yAxis: band.lower, itemStyle: { color: 'rgba(76, 175, 80, 0.08)' } },
      { yAxis: band.upper },
    ]],
  };
}
