export const controls = {
  resources: {
    brand: resource({
      kind: "font",
      default: [
        "https://fonts.gstatic.com/s/notosans/v42/o-0IIpQlx3QUlC5A4PNb4j5Ba_2c7A.ttf",
        "https://fonts.gstatic.com/s/notosans/v42/o-0OIpQlx3QUlC5A4PNr4DRFSfiM7HBj.ttf",
      ],
    }),
  },
};

export const composition = { width: 256, height: 160, fps: 1, duration: 1 };

// A font resource of two files is one family: italic text takes the italic file, and weights and
// widths set the upright file's axes.
export default function Scene() {
  return (
    <View style={{ padding: 8, gap: 6, fontFamily: "brand", color: "#263238" }}>
      <Text style={{ fontSize: 20 }}>
        Upright and <Span style={{ fontStyle: "italic", color: "#e53935" }}>italic</Span>
      </Text>
      <Text style={{ fontSize: 20, fontWeight: 800, color: "#1e88e5" }}>Heavy 800</Text>
      <Text style={{ fontSize: 20, fontStretch: "75%", fontStyle: "italic" }}>Narrow italic</Text>
    </View>
  );
}
