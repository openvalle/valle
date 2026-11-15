export const composition = { width: 640, height: 410, fps: 4, duration: 2 };
export const controls = {
  resources: {
    Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
  },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 640, height: 410, background: "white" }}>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 16,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
          direction: "ltr",
        }}
      >
        ABC אבג 123.
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 64,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
          direction: "rtl",
        }}
      >
        ABC אבג 123.
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 112,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
          direction: "rtl",
          unicodeBidi: "bidi-override",
        }}
      >
        ABC 123.
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 160,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
        }}
      >
        LEFT [<Span style={{ direction: "rtl", unicodeBidi: "isolate" }}>אבג 123</Span>] RIGHT
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 208,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
        }}
      >
        LEFT [<Span style={{ direction: "rtl", unicodeBidi: "isolate-override" }}>abc 123</Span>]
        RIGHT
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 256,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
        }}
      >
        L <Span style={{ direction: "rtl", unicodeBidi: "embed" }}>אבג (123)</Span> R
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 16,
          top: 304,
          width: 350,
          fontFamily: "Latin, Hebrew",
          fontSize: 24,
          lineHeight: "36px",
          color: "#17365d",
          direction: "rtl",
          unicodeBidi: "plaintext",
          whiteSpace: "pre",
        }}
      >
        {"ABC 123\nאבג 123"}
      </Text>
      <View
        style={{
          position: "absolute",
          left: 400,
          top: 20,
          width: 210,
          height: 70,
          direction: "rtl",
          display: "flex",
          gap: 8,
          paddingInline: "12px 4px",
          background: "#f1f5f9",
          borderInlineStart: "4px solid crimson",
          borderStartStartRadius: 16,
        }}
      >
        <View style={{ width: 30, height: 30, background: "#2563eb" }} />
        <View style={{ width: 40, height: 30, background: "#eab308" }} />
      </View>
      <Text
        style={{
          position: "absolute",
          left: 420,
          top: 130,
          width: 160,
          height: 40,
          fontFamily: "Latin",
          fontSize: 24,
          direction: seconds < 1 ? "rtl" : "ltr",
          paddingInlineStart: 12,
          borderInlineStart: "3px solid crimson",
          background: "#f1f5f9",
        }}
      >
        Start 123
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 420,
          top: 210,
          width: 60,
          height: 160,
          fontFamily: "Latin",
          fontSize: 24,
          lineHeight: "40px",
          direction: "rtl",
          writingMode: "vertical-rl",
          paddingInline: "8px 12px",
          borderInlineStart: "2px solid crimson",
          background: "#f1f5f9",
        }}
      >
        ABC 12
      </Text>
      <Text
        style={{
          position: "absolute",
          left: 520,
          top: 210,
          width: 60,
          height: 160,
          fontFamily: "Latin",
          fontSize: 24,
          lineHeight: "40px",
          direction: "ltr",
          writingMode: "vertical-rl",
          paddingInline: "8px 12px",
          borderInlineStart: "2px solid crimson",
          background: "#f1f5f9",
        }}
      >
        ABC 12
      </Text>{" "}
    </View>
  );
}
