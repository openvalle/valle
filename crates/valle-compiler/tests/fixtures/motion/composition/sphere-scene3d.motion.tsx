export const composition = { width: 1920, height: 1080, fps: 30, duration: 1 };
export const controls = { assets: { model: asset({ kind: "model3d", required: true }) } };

export default function FullHdSphere() {
  return <Scene style={{ width: 1920, height: 1080, backgroundColor: "#000000" }}>
    <Scene3D key="stage" style={{ width: 1920, height: 1080 }}
      camera={{ position: [0, 0, 3], target: [0, 0, 0], fov: 45 }}>
      <Mesh key="sphere" src="asset://model" material={{ type: "unlit", color: "#efb343" }} />
    </Scene3D>
  </Scene>;
}
