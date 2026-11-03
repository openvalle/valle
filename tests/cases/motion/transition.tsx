export const composition = { width: 360, height: 232, fps: 4, duration: 1 };

// Transitions between two panels as time drives their progress: built-in kinds with their
// params, one rounded, and one written in gl-transitions' form.
const diagonal = `uniform float tilt; // = 0.5
vec4 transition(vec2 uv) {
  float d = uv.x + (1.0 - uv.y) * tilt;
  float m = step(d, progress * (1.0 + tilt));
  return mix(getFromColor(uv), getToColor(uv), m);
}`;

function Panel({ label, from }: { label: string; from: boolean }) {
  return (
    <View
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: from ? "linear-gradient(135deg, #1e88e5, #00acc1)" : "#ff7043",
        color: "white",
        fontSize: 40,
        fontWeight: 800,
      }}
    >
      <Text>{label}</Text>
    </View>
  );
}

export default function Scene() {
  const { seconds } = useCurrentTime();
  const progress = 0.2 + seconds * 0.6;
  const size = { width: 104, height: 96 };
  const pair = [<Panel label="A" from />, <Panel label="B" from={false} />];
  return (
    <View
      style={{
        display: "flex",
        flexWrap: "wrap",
        width: 360,
        height: 232,
        background: "#fffde7",
        padding: 12,
        gap: 12,
      }}
    >
      <Transition kind="fade" progress={progress} style={size}>
        {pair}
      </Transition>
      <Transition kind="wipeLeft" params={{ softness: 0.2 }} progress={progress} style={size}>
        {pair}
      </Transition>
      <Transition
        kind="circleOpen"
        params={{ centerX: 0.25, centerY: 0.25 }}
        progress={progress}
        style={{ ...size, borderRadius: 16 }}
      >
        {pair}
      </Transition>
      <Transition kind="perlin" params={{ scale: 6 }} progress={progress} style={size}>
        {pair}
      </Transition>
      <Transition
        kind="directionalWarp"
        params={{ directionX: 1, directionY: 1 }}
        progress={progress}
        style={size}
      >
        {pair}
      </Transition>
      <Transition code={diagonal} uniforms={{ tilt: 0.8 }} progress={progress} style={size}>
        {pair}
      </Transition>
    </View>
  );
}
