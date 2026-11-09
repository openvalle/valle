export const composition = { width: 520, height: 240, fps: 4, duration: 1.25 };
const kinds = [
  "fade",
  "wipeLeft",
  "wipeRight",
  "circleOpen",
  "simpleZoom",
  "crossWarp",
  "directionalWarp",
  "linearBlur",
  "dreamyZoom",
  "ripple",
  "flyEye",
  "multiplyBlend",
  "perlin",
];
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        width: 520,
        height: 240,
        background: "#243147",
        display: "flex",
        flexWrap: "wrap",
        alignContent: "flex-start",
      }}
    >
      {kinds.map((kind) => (
        <Transition
          key={kind}
          kind={kind}
          progress={seconds}
          style={{ width: 100, height: 72, margin: 2 }}
        >
          <View
            style={{
              width: "100%",
              height: "100%",
              background: "linear-gradient(135deg, #f0643a, #2271d9)",
            }}
          >
            <View style={{ width: 25, height: 18, background: "#f9e62e", margin: 8 }} />
          </View>
          <View
            style={{
              width: "100%",
              height: "100%",
              background: "linear-gradient(45deg, #6a2cd9, #30e7a0)",
            }}
          >
            <View style={{ width: 16, height: 35, background: "#fff", margin: 15 }} />
          </View>
        </Transition>
      ))}
    </View>
  );
}
