export const composition = { width: 256, height: 416, fps: 1, duration: 1 };

// Font variants: small capitals, all of them, and titling capitals, which the font does not
// have; old-style proportional and tabular figures; fractions with a slashed zero; ordinals;
// superscripts and subscripts; no kerning; no ligatures; the shorthand; a Span back to normal
// capitals; and a feature setting that turns small capitals back off. One Span to a line: Chrome
// rounds each run of text up to 1/64 px, which adds up past the box tolerance over several.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, fontSize: 18 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontVariantCaps: "small-caps" }}>Small Caps 2024</Text>
      <Text style={{ ...row, fontVariantCaps: "all-small-caps" }}>All Small Caps</Text>
      <Text style={{ ...row, fontVariantCaps: "titling-caps" }}>Titling Caps</Text>
      <Text style={{ ...row, fontVariantNumeric: "oldstyle-nums proportional-nums" }}>
        1234567890 1111
      </Text>
      <Text style={{ ...row, fontVariantNumeric: "tabular-nums" }}>1234567890 1111</Text>
      <Text style={{ ...row, fontVariantNumeric: "diagonal-fractions slashed-zero" }}>
        1/2 3/4 10 0
      </Text>
      <Text style={{ ...row, fontVariantNumeric: "ordinal" }}>1st 2nd No</Text>
      <Text style={row}>
        x<Span style={{ fontVariantPosition: "super" }}>2</Span> squared
      </Text>
      <Text style={row}>
        H<Span style={{ fontVariantPosition: "sub" }}>2</Span>O
      </Text>
      <Text style={{ ...row, fontKerning: "none" }}>AVATAR To</Text>
      <Text style={{ ...row, fontVariantLigatures: "none" }}>office fit</Text>
      <Text style={{ ...row, fontVariant: "small-caps tabular-nums" }}>Abc 1111</Text>
      <Text style={{ ...row, fontVariant: "all-small-caps" }}>
        small <Span style={{ fontVariantCaps: "normal", color: "#1e88e5" }}>Normal</Span> small
      </Text>
      <Text style={{ ...row, fontVariantCaps: "small-caps", fontFeatureSettings: "'smcp' 0" }}>
        Not Small
      </Text>
    </View>
  );
}
