export const composition = { width: 96, height: 64, fps: 30, duration: 2 };

export default function GlassFrost() {
  return (
    <Scene style={{ width: 96, height: 64,
      backgroundImage: "repeating-linear-gradient(90deg, #fde047 0px, #fde047 4px, #7c3aed 4px, #7c3aed 8px)" }}>
      <Glass material={{ frost: 5, refraction: 0 }} light={{ intensity: 0 }}
        style={{ position: "absolute", left: 16, top: 12, width: 64, height: 40, borderRadius: 12 }} />
    </Scene>
  );
}
