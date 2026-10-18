export const norm = (v: readonly number[]): number[] => {
  const length = Math.hypot(...v);
  return v.map((channel) => channel / length);
};
export const linear = (v: number): number =>
  v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
