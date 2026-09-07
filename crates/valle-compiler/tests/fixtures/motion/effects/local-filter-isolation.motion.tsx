export const component = "local-filter-isolation";

export const controls = defineControls({
  timing: {
    enterFrames: frames({ default: 0, min: 0 }),
    holdCycleFrames: optionalFrames({ default: null, min: 1 }),
    exitFrames: frames({ default: 0, min: 0 }),
  },
});

const scanlines = defineRepeater({ count: 13, keyPrefix: "scanline" });
const satellites = defineRepeater({ count: 12, keyPrefix: "satellite" });
const ORBIT = path("M 640 176 C 824 176 972 258 972 360 C 972 462 824 544 640 544 C 456 544 308 462 308 360 C 308 258 456 176 640 176 Z");

export default function LocalFilterIsolation(ctx) {
  const t = ctx.hold.progress;
  const pulse = (sin(t * 12.566370614) + 1) * 0.5;
  const blur = 2 + pulse * 14;
  const hue = -24 + pulse * 76;
  const reveal = interpolate(t, [0, 0.16], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#050711" }}>
      {scanlines.map((line) => (
        <View key={line.key} className="absolute" style={{ left: 0, top: 80 + line.index * 48, width: 1280, height: 1, backgroundColor: line.index % 2 === 0 ? "#162036" : "#11182a", opacity: 0.72 }} />
      ))}

      <Text key="kicker" className="absolute" style={{ left: 64, top: 43, fontSize: 15, letterSpacing: 4, color: "#67e8f9", opacity: reveal }}>LOCAL EFFECT SCOPE</Text>
      <Text key="title" className="absolute" style={{ left: 60, top: 76, width: 1120, fontSize: 46, color: "#f8fafc", opacity: reveal }}>One filtered subtree. Everything else stays sharp.</Text>

      <Path key="orbit-line" d={ORBIT} fill="none" stroke="#334155b3" strokeWidth="1" strokeDasharray="6 12" />
      {satellites.map((dot) => (
        <View key={dot.key} className="absolute" style={{ width: 7 + (1 - dot.progress) * 7, height: 7 + (1 - dot.progress) * 7, borderRadius: 10, backgroundColor: dot.index % 2 === 0 ? "#67e8f9" : "#c4b5fd", opacity: 0.24 + (1 - dot.progress) * 0.56, motionPath: follow(ORBIT, trail(t, dot.index, { gap: 0.052, mode: "wrap" })) }} />
      ))}

      <View key="left-reference" className="absolute" style={{ borderStyle: "solid", left: 92, top: 253, width: 258, height: 222, borderRadius: 28, backgroundColor: "#0b1222ee", borderWidth: 1, borderColor: "#334155" }}>
        <Text key="left-index" className="absolute" style={{ left: 24, top: 23, fontSize: 13, letterSpacing: 3, color: "#64748b" }}>01 / REFERENCE</Text>
        <Text key="left-glyph" className="absolute" style={{ left: 24, top: 57, fontSize: 84, color: "#f8fafc" }}>A</Text>
        <View key="left-rule" className="absolute" style={{ left: 25, top: 166, width: 204, height: 3, backgroundColor: "#22d3ee" }} />
        <Text key="left-caption" className="absolute" style={{ left: 24, top: 180, fontSize: 14, color: "#94a3b8" }}>SIBLING / SHARP</Text>
      </View>

      <View key="filtered-target" className="absolute" style={{ borderStyle: "solid", left: 443, top: 218, width: 394, height: 292, borderRadius: 44, backgroundColor: "#11182b", borderWidth: 2, borderColor: "#67e8f9", filter: `blur(${blur}px) hue-rotate(${hue}deg) saturate(1.55) drop-shadow(0px 0px 26px #22d3eeaa)` }}>
        <View key="target-core" className="absolute" style={{ borderStyle: "solid", left: 116, top: 52, width: 162, height: 162, borderRadius: 48, backgroundColor: "#7c3aed", borderWidth: 2, borderColor: "#f8fafc", transform: `scale(${0.9 + pulse * 0.16})` }}>
          <Text key="target-glyph" className="absolute" style={{ left: 40, top: 21, fontSize: 92, color: "#ffffff" }}>V</Text>
        </View>
        <Text key="target-caption" className="absolute" style={{ left: 0, top: 244, width: 394, textAlign: "center", fontSize: 14, letterSpacing: 3, color: "#e0f2fe" }}>FILTERED SUBTREE</Text>
      </View>

      <View key="right-reference" className="absolute" style={{ borderStyle: "solid", left: 930, top: 253, width: 258, height: 222, borderRadius: 28, backgroundColor: "#0b1222ee", borderWidth: 1, borderColor: "#334155" }}>
        <Text key="right-index" className="absolute" style={{ left: 24, top: 23, fontSize: 13, letterSpacing: 3, color: "#64748b" }}>03 / REFERENCE</Text>
        <Text key="right-glyph" className="absolute" style={{ left: 24, top: 57, fontSize: 84, color: "#f8fafc" }}>B</Text>
        <View key="right-rule" className="absolute" style={{ left: 25, top: 166, width: 204, height: 3, backgroundColor: "#a78bfa" }} />
        <Text key="right-caption" className="absolute" style={{ left: 24, top: 180, fontSize: 14, color: "#94a3b8" }}>SIBLING / SHARP</Text>
      </View>

      <Text key="footer" className="absolute" style={{ left: 64, top: 641, width: 1152, fontSize: 16, letterSpacing: 2, color: "#94a3b8", opacity: reveal }}>CURRENT BASELINE · CSS FILTER GROUPS ALREADY PROVIDE NODE-LOCAL OFFSCREEN COMPOSITION</Text>
    </Scene>
  );
}
