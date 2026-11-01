export const composition = { width: 256, height: 320, fps: 1, duration: 1 };

// OpenType features: small capitals, and capitals to small capitals too; old-style and
// proportional figures against tabular ones; fractions and a slashed zero; ordinals; no kerning,
// and no ligatures; a stylistic set; a Span that turns one off inside text that turns it on; and
// letter spacing, which turns ligatures off.
export default function Scene() {
  const row = { background: "#eceff1", marginBottom: 4, fontSize: 18 };
  return (
    <View style={{ padding: 4, color: "#263238" }}>
      <Text style={{ ...row, fontFeatureSettings: "'smcp'" }}>Small Caps 2024</Text>
      <Text style={{ ...row, fontFeatureSettings: "'smcp', 'c2sc'" }}>All Small Caps</Text>
      <Text style={{ ...row, fontFeatureSettings: "'onum', 'pnum'" }}>1234567890 1111</Text>
      <Text style={{ ...row, fontFeatureSettings: "'tnum' on" }}>1234567890 1111</Text>
      <Text style={{ ...row, fontFeatureSettings: "'frac', 'zero'" }}>1/2 3/4 10 0</Text>
      <Text style={{ ...row, fontFeatureSettings: "'ordn'" }}>1st 2nd No</Text>
      <Text style={row}>AVATAR To fi fl</Text>
      <Text style={{ ...row, fontFeatureSettings: "'kern' 0, 'liga' off" }}>AVATAR To fi fl</Text>
      <Text style={{ ...row, fontFeatureSettings: "'ss03', 'salt'" }}>gilt ag 1 Iq</Text>
      <Text style={{ ...row, fontFeatureSettings: "'smcp'" }}>
        small <Span style={{ fontFeatureSettings: "normal", color: "#1e88e5" }}>plain</Span> small
      </Text>
      <Text style={{ ...row, letterSpacing: 1 }}>fi fl office</Text>
    </View>
  );
}
