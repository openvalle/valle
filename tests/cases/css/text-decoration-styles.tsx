export const composition = { width: 256, height: 380, fps: 1, duration: 1 };

// Decoration styles: double lines under, over and through; dotted, square and round; dashed,
// thin and thick; wavy, of the font's thickness and set thicker, in a color, under glyphs that
// break it, over and through; a span's own style; and skip-ink off.
export default function Scene() {
  const row = { fontSize: 20, lineHeight: "34px", color: "#263238" };
  return (
    <View style={{ padding: 6 }}>
      <Text style={{ ...row, textDecoration: "underline double" }}>Double under gyp</Text>
      <Text style={{ ...row, textDecoration: "overline line-through double 2px #1e88e5" }}>
        Over and through
      </Text>
      <Text style={{ ...row, textDecoration: "underline dotted" }}>Dotted jumpy line</Text>
      <Text style={{ ...row, textDecoration: "underline dotted 4px #e53935" }}>Round dots gyp</Text>
      <Text style={{ ...row, textDecoration: "underline dashed" }}>Dashed quietly</Text>
      <Text style={{ ...row, textDecoration: "underline dashed 3px #43a047" }}>Thick dashes</Text>
      <Text style={{ ...row, textDecoration: "underline wavy" }}>Wavy typography</Text>
      <Text style={{ ...row, textDecoration: "underline wavy 3px #e53935" }}>Spelling gyp</Text>
      <Text style={{ ...row, textDecoration: "overline line-through wavy #8e24aa" }}>
        Over and through
      </Text>
      <Text style={{ ...row, textDecoration: "underline" }}>
        Solid <Span style={{ textDecoration: "underline wavy #1e88e5" }}>span</Span> after
      </Text>
      <Text
        style={{
          ...row,
          textDecoration: "underline dotted 2px",
          textDecorationSkipInk: "none",
        }}
      >
        No skipping gyp
      </Text>
    </View>
  );
}
