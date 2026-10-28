export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
const hdr = `
uniform float phase;
float4 valle_main(float2 uv) {
  return float4(8.0 + phase, -4.0, 0.5, 0.5);
}`;
export default function WorkingHdr() {
  const time = useCurrentTime();
  return <Scene style={{ width: 320, height: 180 }}>
    <ShaderLayer code={hdr} uniforms={{ phase: Math.floor(time.seconds * 30) / 64 }} style={{ position: "absolute", left: 16, top: 16, width: 128, height: 96 }} />
    <View style={{ position: "absolute", left: 180, top: 24, width: 96, height: 72, backgroundColor: "#80c04080" }} />
  </Scene>;
}
