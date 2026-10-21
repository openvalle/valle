export const composition = { width: 256, height: 224, fps: 30, duration: 2 };

// Refraction over a horizontal ramp: a crease in the rim's slope would be a step in brightness
// across the corner's diagonal.
export default function GlassCorner() {
  return (
    <Scene style={{ width: 256, height: 224,
      backgroundImage: "linear-gradient(90deg, #000000, #ffffff)" }}>
      <Glass material={{ bevel: 24, refraction: 16 }} light={{ intensity: 0 }}
        style={{ position: "absolute", left: 64, top: 32, width: 160, height: 160,
          borderRadius: 48 }} />
    </Scene>
  );
}
