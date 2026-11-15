export const composition = { width: 300, height: 340, fps: 1, duration: 1 };
export const controls = {
  resources: { Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }) },
};
export default function Scene() {
  return (
    <View style={{ position: "relative", width: 300, height: 340, backgroundColor: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 8,
          width: 268,
          fontFamily: "Hebrew",
          fontSize: 40,
          lineHeight: "50px",
          color: "#17365d",
          fontWeight: 400,
          fontStyle: "normal",
          fontSynthesis: "none",
        }}
      >
        אבגדה
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 62,
          width: 268,
          fontFamily: "Hebrew",
          fontSize: 40,
          lineHeight: "50px",
          color: "#17365d",
          fontWeight: 700,
          fontStyle: "normal",
          fontSynthesis: "weight style",
        }}
      >
        אבגדה
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 116,
          width: 268,
          fontFamily: "Hebrew",
          fontSize: 40,
          lineHeight: "50px",
          color: "#17365d",
          fontWeight: 700,
          fontStyle: "normal",
          fontSynthesis: "none",
        }}
      >
        אבגדה
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 170,
          width: 268,
          fontFamily: "Hebrew",
          fontSize: 40,
          lineHeight: "50px",
          color: "#17365d",
          fontWeight: 400,
          fontStyle: "italic",
          fontSynthesis: "style",
        }}
      >
        אבגדה
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 224,
          width: 268,
          fontFamily: "Hebrew",
          fontSize: 40,
          lineHeight: "50px",
          color: "#17365d",
          fontWeight: 400,
          fontStyle: "italic",
          fontSynthesis: "none",
        }}
      >
        אבגדה
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 278,
          width: 268,
          fontFamily: "Hebrew",
          fontSize: 40,
          lineHeight: "50px",
          color: "#17365d",
          fontWeight: 700,
          fontStyle: "italic",
          fontSynthesis: "weight",
        }}
      >
        אבגדה
      </Text>
    </View>
  );
}
