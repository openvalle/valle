export const composition = { width: 220, height: 100, fps: 30, duration: 2 };

// Frame 0 leaves 50 px between the circles; frame 30 leaves 8, which the field bridges.
export default function GlassFieldBridge() {
  const time = useCurrentTime();
  return (
    <Scene style={{ width: 220, height: 100, backgroundColor: "#000000" }}>
      <GlassField material={{ refraction: 0, tint: "#ff0000" }} light={{ intensity: 0 }}
        merge={{ distance: 24 }}>
        <Glass style={{ position: "absolute", left: -10 + 84 * time.progress, top: 10,
          width: 80, height: 80, borderRadius: "50%" }} />
        <Glass style={{ position: "absolute", left: 120, top: 10, width: 80, height: 80,
          borderRadius: "50%" }} />
      </GlassField>
    </Scene>
  );
}
