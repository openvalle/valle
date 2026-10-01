export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const GLYPH = textOutline("V", { fontSize: 220, fontWeight: 800, align: "center" });
export default function Extruded(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Scene3D key="s" style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }}
        camera={{ position: [0, 0, 420], target: [0, 0, 0] }}>
        <AmbientLight intensity={0.4} />
        <DirectionalLight direction={[0.5, 1, 1]} intensity={2} />
        <Mesh key="v" geometry={extrude(GLYPH.path, { depth: 60, bevel: 6 })}
          material={{ type: "pbr", color: "#8b7bff", metallic: 0, roughness: 0.4 }}
          position={[0, -80, 0]} rotation={[0, ctx.seconds * 40, 0]} />
      </Scene3D>
    </Scene>
  );
}
