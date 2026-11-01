export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// Font variations: the thinnest and heaviest weights; an axis over the weight; the narrowest
// width, and a width with a weight; an underline that skips the ink of heavy glyphs; a Span back
// to normal in heavy text; an ellipsis in the text's weight; an axis the font does not have; and
// a weight past the axis, which stops at its end.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, fontSize: 18 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontVariationSettings: "'wght' 100" }}>Thin weight</Text>
      <Text style={{ ...row, fontVariationSettings: "'wght' 900" }}>Black weight</Text>
      <Text style={{ ...row, fontWeight: 900, fontVariationSettings: "'wght' 300" }}>
        Light over black
      </Text>
      <Text style={{ ...row, fontVariationSettings: "'wdth' 62.5" }}>Condensed width</Text>
      <Text style={{ ...row, fontVariationSettings: "'wdth' 75, 'wght' 650" }}>
        Narrow and bold
      </Text>
      <Text style={{ ...row, fontVariationSettings: "'wght' 900", textDecoration: "underline" }}>
        Gypsy jogging
      </Text>
      <Text style={{ ...row, fontVariationSettings: "'wght' 900" }}>
        Black <Span style={{ fontVariationSettings: "normal", color: "#1e88e5" }}>regular</Span>
      </Text>
      <Text
        style={{
          ...row,
          width: 140,
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
          fontVariationSettings: "'wght' 800",
        }}
      >
        The quick brown fox jumps
      </Text>
      <Text style={{ ...row, fontVariationSettings: "'XOPQ' 50" }}>No such axis</Text>
      <Text style={{ ...row, fontVariationSettings: "'wght' 1200" }}>Past the end</Text>
    </View>
  );
}
