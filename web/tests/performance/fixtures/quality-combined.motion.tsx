export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), { segments: 16 });
const PANEL = extrude(path("M -3 -2 L 3 -2 L 3 2 L -3 2 Z"), { depth: 0.05 });
export default function Combined() {
  const t = useCurrentTime();
  const frame = Math.floor(t.seconds * 30);
  return <Scene style={{ width: 320, height: 180 }}>
    <View style={{ position: "absolute", left: 8 + frame, top: 8, width: 96, height: 58 }}>
      <Shutter samples={8} angle={180}>
        <View style={{ position: "absolute", left: 4, top: 4, width: 22, height: 20, backgroundColor: "#70c0e080", opacity: 0.75 }} />
        <View style={{ position: "absolute", left: 30, top: 4, width: 22, height: 20, backgroundColor: "#70c0e080", opacity: 0.75 }} />
        <View style={{ position: "absolute", left: 56, top: 4, width: 22, height: 20, backgroundColor: "#70c0e080", opacity: 0.75 }} />
      </Shutter>
      <View style={{ position: "absolute", left: 12, top: 30, width: 40, height: 10, backgroundColor: "#e0a060", filter: "blur(16px)" }} />
    </View>
    <Scene3D key="depth" style={{ position: "absolute", left: 180, top: 8, width: 128, height: 96 }} pbr={{ shadows: true }} camera={{ position: [0, 0, 6], target: [0, 0, 0], fov: 45, depthOfField: { focusDistance: 6, maxBlurRadius: 6 } }}>
      <AmbientLight intensity={0.3} />
      <DirectionalLight direction={[1, 1, 2]} intensity={1.5} />
      <Mesh key="panel" geometry={PANEL} position={[0, 0, -0.2]} material={{ type: "pbr", color: "#b4b9bf", roughness: 0.9 }} />
      <Mesh key="ball" geometry={BALL} position={[Math.sin(t.seconds) * 0.7, 0, 0.6]} scale={[0.5, 0.5, 0.5]} material={{ type: "pbr", color: "#d07040", roughness: 0.4 }} />
    </Scene3D>
    <Text style={{ position: "absolute", left: 12, top: 128, fontSize: 8, color: "#c0c0c0" }}>VALLE 012345 small text</Text>
    <View style={{ position: "absolute", left: 12, top: 152, width: 296, height: 20, backgroundImage: "linear-gradient(90deg, #000000, #202020)" }} />
  </Scene>;
}
