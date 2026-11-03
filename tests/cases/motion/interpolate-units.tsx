export const composition = { width: 256, height: 160, fps: 4, duration: 1 };

// Values with units over time: a width from pixels to a percentage, a turn written in two units,
// colors mixed as a transition mixes them and the long way round oklch's hue, and a radius.
export default function Scene() {
  const { seconds } = useCurrentTime();
  const at = (outputs, options) => interpolate(seconds, [0, 1], outputs, options);
  return (
    <View style={{ display: "flex", flexDirection: "column", padding: 8, gap: 8 }}>
      <View style={{ height: 16, width: at(["24px", "90%"]), background: "#1e88e5" }} />
      <View style={{ display: "flex", gap: 8 }}>
        <View
          style={{
            width: 48,
            height: 48,
            background: at(["#e53935", "#1e88e5"]),
            rotate: at(["0deg", "0.25turn"]),
            borderRadius: at(["0px", "24px"]),
          }}
        />
        <View
          style={{
            width: 48,
            height: 48,
            background: at(["oklch(0.7 0.15 30)", "oklch(0.7 0.15 90)"], {
              colorSpace: "oklch",
              hue: "longer",
            }),
          }}
        />
        <View
          style={{
            width: 48,
            height: 48,
            background: at(["red", "rgb(0 0 255 / 0.2)"], { easing: "ease-in" }),
          }}
        />
      </View>
    </View>
  );
}
