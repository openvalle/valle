export const composition = { width: 240, height: 136, fps: 30, duration: 2 };

export default function GlassOptics() {
  return (
    <Scene style={{ width: 240, height: 136,
      backgroundImage: "repeating-linear-gradient(60deg, #f43f5e 0px, #f43f5e 6px, #22d3ee 6px, #22d3ee 12px)" }}>
      <Glass material={{ refraction: 10, dispersion: 0.4, frost: 1, tint: "#ffffff18" }}
        style={{ position: "absolute", left: 12, top: 18, width: 100, height: 100, borderRadius: 32,
          boxShadow: "0 6px 12px #00000066" }} />
      <Glass path="M 40 0 L 80 70 L 0 70 Z" material={{ bevel: 18, refraction: 10 }}
        light={{ direction: point(1, -0.5), intensity: 0.9 }}
        style={{ position: "absolute", left: 140, top: 20, width: 80, height: 70,
          transform: "perspective(300px) rotateY(30deg)" }} />
    </Scene>
  );
}
