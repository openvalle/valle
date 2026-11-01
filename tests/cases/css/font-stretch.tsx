export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Font stretch: condensed, 62.5%, the narrowest width of the font; ultra-condensed and expanded,
// which stop at the ends of its width axis; semi-condensed and heavy; a variation setting over
// the stretch; and a condensed Span in text of normal width.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, fontSize: 18 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontStretch: "condensed" }}>Condensed text</Text>
      <Text style={{ ...row, fontStretch: "62.5%" }}>At 62.5 percent</Text>
      <Text style={{ ...row, fontStretch: "ultra-condensed" }}>Ultra-condensed</Text>
      <Text style={{ ...row, fontStretch: "expanded" }}>Expanded text</Text>
      <Text style={{ ...row, fontStretch: "semi-condensed", fontWeight: 800 }}>
        Semi-condensed and heavy
      </Text>
      <Text style={{ ...row, fontStretch: "condensed", fontVariationSettings: "'wdth' 100" }}>
        Settings over the stretch
      </Text>
      <Text style={row}>
        Normal <Span style={{ fontStretch: "condensed", color: "#1e88e5" }}>condensed</Span>
      </Text>
    </View>
  );
}
