export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function ParticleForces(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <GeometryBatch key="p" geometry="circle"
        positions={particles(ctx.localFrame, ctx.fps, {
          seed: 7, count: 400, emitter: rect(300, 300, 40, 4), birth: { interval: 0.005 }, lifetime: 2,
          velocity: { x: [-40, 40], y: [-160, -120] }, gravity: point(0, 0), loop: true,
          forces: [curlNoise({ seed: 3, scale: 0.012, strength: 220 }), drag(0.8)],
        })}
        sizes={[3, 3]} fills={["#ffffff", "#ffffff"]}
        style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
    </Scene>
  );
}
