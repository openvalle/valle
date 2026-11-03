export const composition = { width: 256, height: 112, fps: 4, duration: 1 };

// Counters as text: a number with grouping and two decimals, a percent, and a padded timer.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const line = { fontSize: 20, color: "#263238" };
  return (
    <View style={{ display: "flex", flexDirection: "column", padding: 8, gap: 4 }}>
      <Text style={line}>{formatNumber(seconds * 1234567.891, { decimals: 2 })}</Text>
      <Text style={line}>{formatPercent(seconds * 0.875, { decimals: 1 })}</Text>
      <Text style={line}>{`00:${padNumber(seconds * 40, { width: 2 })}`}</Text>
    </View>
  );
}
