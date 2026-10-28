export const composition = { width: 1920, height: 1080, fps: 30, duration: 2 };
const PLANE = extrude(path("M -10 -6 L 10 -6 L 10 6 L -10 6 Z"), { depth: 0.01 });
const STRIPE = extrude(path("M -0.08 -6 L 0.08 -6 L 0.08 6 L -0.08 6 Z"), { depth: 0.01 });
const BALL = lathe(path("M 0 -1 C 0.5523 -1 1 -0.5523 1 0 C 1 0.5523 0.5523 1 0 1"), { segments: 24 });
export default function DepthOfField() {
  const time = useCurrentTime();
  return <Scene style={{ width: 1920, height: 1080, backgroundColor: "#101824" }}>
    <Scene3D key="stage" style={{ width: 1920, height: 1080 }} camera={{ position: [0, 0, 9], target: [0, 0, 0], fov: 45, depthOfField: { focusDistance: 9, maxBlurRadius: 16 } }}>
      <Mesh key="plane" geometry={PLANE} material={{ type: "unlit", color: "#99b5cf" }} />
      <Mesh key="stripe0" geometry={STRIPE} position={[-2 + time.seconds * 0.1, 0, 0.03]} material={{ type: "unlit", color: "#12253e" }} />
      <Mesh key="stripe1" geometry={STRIPE} position={[-0.8 + time.seconds * 0.1, 0, 0.03]} material={{ type: "unlit", color: "#e6d8ba" }} />
      <Mesh key="stripe2" geometry={STRIPE} position={[0.4 + time.seconds * 0.1, 0, 0.03]} material={{ type: "unlit", color: "#12253e" }} />
      <Mesh key="stripe3" geometry={STRIPE} position={[1.6 + time.seconds * 0.1, 0, 0.03]} material={{ type: "unlit", color: "#e6d8ba" }} />

    </Scene3D>
  </Scene>;
}
