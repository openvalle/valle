export const composition = { width: 560, height: 210, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Sans: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 560, height: 210, backgroundColor: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 12,
          top: 8,
          fontFamily: "Sans",
          fontSize: 32,
          lineHeight: "52px",
          fontVariantCaps: "petite-caps",
          color: "#17365d",
        }}
      >
        Abc DEf ß
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 12,
          top: 72,
          fontFamily: "Sans",
          fontSize: 32,
          lineHeight: "52px",
          fontVariantCaps: "all-petite-caps",
          color: "#17365d",
        }}
      >
        Abc DEf ß
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 12,
          top: 136,
          fontFamily: "Sans",
          fontSize: 32,
          lineHeight: "52px",
          fontVariantCaps: "unicase",
          color: "#17365d",
        }}
      >
        Abc DEf ß
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 292,
          top: 8,
          fontFamily: "Sans",
          fontSize: 32,
          lineHeight: "52px",
          fontFeatureSettings: '"smcp" 1',
          color: "#17365d",
        }}
      >
        Abc DEf ß
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 292,
          top: 72,
          fontFamily: "Sans",
          fontSize: 32,
          lineHeight: "52px",
          fontFeatureSettings: '"smcp" 1, "c2sc" 1',
          color: "#17365d",
        }}
      >
        Abc DEf ß
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 292,
          top: 136,
          fontFamily: "Sans",
          fontSize: 32,
          lineHeight: "52px",
          fontFeatureSettings: '"c2sc" 1',
          color: "#17365d",
        }}
      >
        Abc DEf ß
      </Text>
    </View>
  );
}
