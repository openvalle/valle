export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export const component = "local-filter-isolation";

export const controls = defineControls({
  timing: { enterDuration: 0, exitDuration: 0 },
});

const scanlines = defineRepeater({ count: 13, keyPrefix: "scanline" });
const satellites = defineRepeater({ count: 12, keyPrefix: "satellite" });
const ORBIT = path("M 960 264 C 1236 264 1458 387 1458 540 C 1458 693 1236 816 960 816 C 684 816 462 693 462 540 C 462 387 684 264 960 264 Z");

export default function LocalFilterIsolation(ctx) {
  const t = ctx.hold.progress;
  const pulse = (sin(t * 12.566370614) + 1) * 0.5;
  const blur = 2 + pulse * 14;
  const hue = -24 + pulse * 76;
  const reveal = interpolate(t, [0, 0.16], [0, 1], { easing: "easeOut" });

  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#050711" }}>
      {scanlines.map((line) => (
        <View key={line.key} className="absolute" style={{ left: 0, top: 120 + line.index * 72, width: 1920, height: 1.5, backgroundColor: line.index % 2 === 0 ? "#162036" : "#11182a", opacity: 0.72 }} />
      ))}

      <Text key="kicker" className="absolute" style={{ left: 96, top: 64.5, fontSize: 22.5, letterSpacing: 6, color: "#67e8f9", opacity: reveal }}>LOCAL EFFECT SCOPE</Text>
      <Text key="title" className="absolute" style={{ left: 90, top: 114, width: 1680, fontSize: 69, color: "#f8fafc", opacity: reveal }}>One filtered subtree. Everything else stays sharp.</Text>

      <Path key="orbit-line" d={ORBIT} fill="none" stroke="#334155b3" strokeWidth="1.5" strokeDasharray="6 12" />
      {satellites.map((dot) => (
        <View key={dot.key} className="absolute" style={{ width: 10.5 + (1 - dot.progress) * 10.5, height: 10.5 + (1 - dot.progress) * 10.5, borderRadius: 15, backgroundColor: dot.index % 2 === 0 ? "#67e8f9" : "#c4b5fd", opacity: 0.24 + (1 - dot.progress) * 0.56, motionPath: follow(ORBIT, trail(t, dot.index, { gap: 0.052, mode: "wrap" })) }} />
      ))}

      <View key="left-reference" className="absolute" style={{ borderStyle: "solid", left: 138, top: 379.5, width: 387, height: 333, borderRadius: 42, backgroundColor: "#0b1222ee", borderWidth: 1.5, borderColor: "#334155" }}>
        <Text key="left-index" className="absolute" style={{ left: 36, top: 34.5, fontSize: 19.5, letterSpacing: 4.5, color: "#64748b" }}>01 / REFERENCE</Text>
        <Text key="left-glyph" className="absolute" style={{ left: 36, top: 85.5, fontSize: 126, color: "#f8fafc" }}>A</Text>
        <View key="left-rule" className="absolute" style={{ left: 37.5, top: 249, width: 306, height: 4.5, backgroundColor: "#22d3ee" }} />
        <Text key="left-caption" className="absolute" style={{ left: 36, top: 270, fontSize: 21, color: "#94a3b8" }}>SIBLING / SHARP</Text>
      </View>

      <View key="filtered-target" className="absolute" style={{ borderStyle: "solid", left: 664.5, top: 327, width: 591, height: 438, borderRadius: 66, backgroundColor: "#11182b", borderWidth: 3, borderColor: "#67e8f9", filter: `blur(${blur}px) hue-rotate(${hue}deg) saturate(1.55) drop-shadow(0px 0px 39px #22d3eeaa)` }}>
        <View key="target-core" className="absolute" style={{ borderStyle: "solid", left: 174, top: 78, width: 243, height: 243, borderRadius: 72, backgroundColor: "#7c3aed", borderWidth: 3, borderColor: "#f8fafc", transform: `scale(${0.9 + pulse * 0.16})` }}>
          <Text key="target-glyph" className="absolute" style={{ left: 60, top: 31.5, fontSize: 138, color: "#ffffff" }}>V</Text>
        </View>
        <Text key="target-caption" className="absolute" style={{ left: 0, top: 366, width: 591, textAlign: "center", fontSize: 21, letterSpacing: 4.5, color: "#e0f2fe" }}>FILTERED SUBTREE</Text>
      </View>

      <View key="right-reference" className="absolute" style={{ borderStyle: "solid", left: 1395, top: 379.5, width: 387, height: 333, borderRadius: 42, backgroundColor: "#0b1222ee", borderWidth: 1.5, borderColor: "#334155" }}>
        <Text key="right-index" className="absolute" style={{ left: 36, top: 34.5, fontSize: 19.5, letterSpacing: 4.5, color: "#64748b" }}>03 / REFERENCE</Text>
        <Text key="right-glyph" className="absolute" style={{ left: 36, top: 85.5, fontSize: 126, color: "#f8fafc" }}>B</Text>
        <View key="right-rule" className="absolute" style={{ left: 37.5, top: 249, width: 306, height: 4.5, backgroundColor: "#a78bfa" }} />
        <Text key="right-caption" className="absolute" style={{ left: 36, top: 270, fontSize: 21, color: "#94a3b8" }}>SIBLING / SHARP</Text>
      </View>

      <Text key="footer" className="absolute" style={{ left: 96, top: 961.5, width: 1728, fontSize: 24, letterSpacing: 3, color: "#94a3b8", opacity: reveal }}>CURRENT BASELINE · CSS FILTER GROUPS ALREADY PROVIDE NODE-LOCAL OFFSCREEN COMPOSITION</Text>
    </Scene>
  );
}
