export const composition = { width: 960, height: 540, fps: 30, duration: 1 };

// Curved bands make actual backdrop displacement visible; the foreground stays undistorted.
const BACKGROUND = `uniform float t;
float4 valle_main(float2 uv) {
  float2 p = uv * float2(1.7778, 1.0);
  float wave = p.x + 0.24 * sin(4.0 * p.y + t * 0.5) + 0.12 * sin(p.y * 9.0);
  float band = 0.5 + 0.5 * sin(wave * 11.0);
  float3 cool = mix(float3(0.035, 0.14, 0.25), float3(0.12, 0.68, 0.72), band);
  float3 warm = mix(float3(0.34, 0.07, 0.14), float3(0.97, 0.52, 0.30), band);
  float3 color = mix(cool, warm, smoothstep(0.55, 1.4, p.x));
  float line = 1.0 - smoothstep(0.012, 0.026, abs(fract(wave * 3.0) - 0.5));
  return float4(mix(color, float3(0.96, 0.94, 0.83), line * 0.7), 1.0);
}`;

export default function LiquidGlass() {
  const { seconds: t } = useCurrentTime();
  return (
    <Scene style={{ width: 960, height: 540 }}>
      <ShaderLayer code={BACKGROUND} uniforms={{ t }} style={{ width: 960, height: 540 }} />
      <Glass material={{ bevel: 24, refraction: 36, frost: 1, dispersion: 0.06 }}
        style={{ position: "absolute", left: 84, top: 100, width: 320, height: 248,
          borderRadius: 44, translate: point(12 * Math.sin(t * 3), 8 * Math.cos(t * 3)),
          boxShadow: "0 12px 28px #05142125" }}>
        <View style={{ position: "absolute", left: 38, top: 40, width: 48, height: 48,
          borderRadius: 15, backgroundColor: "#ffffffcc" }} />
        <View style={{ position: "absolute", left: 108, top: 48, width: 162, height: 9,
          borderRadius: 5, backgroundColor: "#ffffffdd" }} />
        <View style={{ position: "absolute", left: 108, top: 70, width: 106, height: 6,
          borderRadius: 3, backgroundColor: "#ffffff80" }} />
        <View style={{ position: "absolute", left: 38, top: 138, width: 244, height: 4,
          borderRadius: 2, backgroundColor: "#ffffff66" }} />
        <View style={{ position: "absolute", left: 38, top: 168, width: 176, height: 4,
          borderRadius: 2, backgroundColor: "#ffffff66" }} />
      </Glass>
      <Glass material={{ bevel: 24, refraction: 30, frost: 5, saturation: 1.12, tint: "#e2efff0c" }}
        style={{ position: "absolute", left: 542, top: 104, width: 230, height: 230,
          borderRadius: 115, translate: point(-10 * Math.sin(t * 3), -8 * Math.cos(t * 3)),
          boxShadow: "0 12px 24px #05142122" }} />
      <GlassField material={{ bevel: 16, refraction: 28, frost: 1, dispersion: 0.04 }} merge={{ distance: 24 }}>
        <Glass style={{ position: "absolute", left: 150, top: 410, width: 220, height: 68, borderRadius: 34 }} />
        <Glass style={{ position: "absolute", left: 384, top: 410, width: 68, height: 68, borderRadius: 34,
          translate: point(-12 * Math.sin(t * 3), 0) }} />
      </GlassField>
      <Glass style={{ position: "absolute", left: 630, top: 410, width: 160, height: 68, borderRadius: 34 }} />
    </Scene>
  );
}
