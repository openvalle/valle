export const composition = { width: 280, height: 180, fps: 4, duration: 2 };
const OPACITY = [1, 0.5, 0, 1, 1, 0.5, 1, 1];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const i = Math.floor(seconds * 4);
  return (
    <View key="canvas" style={{ width: 280, height: 180, background: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          width: 256,
          height: 156,
          background: "#4080c0",
        }}
      >
        {[0, 1, 2, 3, 4, 5, 6, 7].map((j) => (
          <View
            style={{
              position: "absolute",
              left: j * 32,
              width: 16,
              height: 156,
              background: "#e0b060",
            }}
          />
        ))}
      </View>
      <View
        key="layer"
        style={{
          position: "absolute",
          left: 40,
          top: 32,
          width: 192,
          height: 112,
          opacity: OPACITY[i],
          display: i === 6 ? "none" : "block",
          visibility: i === 7 ? "hidden" : "visible",
          filter: i === 3 ? "blur(3px)" : "none",
          transform: i === 4 ? "rotate(8deg)" : "none",
          clipPath: i === 4 ? "circle(45%)" : "none",
          maskImage: i === 5 ? "linear-gradient(to right,black,transparent)" : "none",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: 192,
            height: 112,
            backdropFilter: "invert(1)",
            mixBlendMode: "multiply",
            background: "rgb(128 192 64 / 50%)",
          }}
        >
          <View
            style={{
              position: "absolute",
              left: 48,
              top: 28,
              width: 96,
              height: 56,
              background: "#40d080",
            }}
          />
        </View>
      </View>
    </View>
  );
}
