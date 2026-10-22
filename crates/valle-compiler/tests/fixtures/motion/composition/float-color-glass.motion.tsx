export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function FloatGlass() { const time = useCurrentTime();
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#172536" }}>
      <Glass key="lens"
        material={{ frost: 6,
          tint: interpolate(time.progress, [0, 1], ["#2140ff", "#ffd000"],
            { colorSpace: "oklch" }) }}
        style={{ position: "absolute", left: 100, top: 70, width: 440, height: 220,
          borderRadius: 28 }} />
    </Scene>
  );
}
