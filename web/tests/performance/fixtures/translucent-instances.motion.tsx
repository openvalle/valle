export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const POSITIONS = Array.from({ length: 4096 }, (_, i) => point((i % 64) * 9, Math.floor(i / 64) * 5));

export default function TranslucentInstances() {
  const time = useCurrentTime();
  return (
    <Scene style={{ width: 640, height: 360, backgroundColor: "#18202a" }}>
      <GeometryBatch
        geometry="rect"
        positions={POSITIONS}
        sizes={point(4, 3)}
        fills="#4ba8e5"
        opacities={0.63}
        style={{ position: "absolute", left: 18 + time.seconds * 2, top: 16, width: 604, height: 336 }}
      />
    </Scene>
  );
}
