// error: errors/format.tsx:6: TypeError: formatNumber takes decimals and grouping…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const { seconds } = useCurrentTime();
  return <Text>{formatNumber(seconds, { digits: 2 })}</Text>;
}
