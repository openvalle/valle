export const composition = { width: 520, height: 260, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "/tests/resources/svg/pixels.png" }) },
};
export default function Scene() {
  return (
    <View
      style={{
        width: 520,
        height: 260,
        background: "white",
        fontSize: 20,
        lineHeight: "32px",
        color: "#17365d",
      }}
    >
      <Text key="row0" style={{ position: "absolute", left: 12, top: 12, width: 496, height: 64 }}>
        A
        <View
          style={{
            display: "inline-block",
            padding: "2px 6px",
            border: "2px solid #c87816",
            background: "#ffe7ad",
          }}
        >
          <Text>badge</Text>
        </View>
        B{" "}
        <Image
          src="resource://picture"
          style={{
            width: 40,
            height: 30,
            margin: "0 6px",
            imageRendering: "pixelated",
            verticalAlign: "middle",
          }}
        />{" "}
        <svg viewBox="0 0 40 40" style={{ width: 40, height: 40, verticalAlign: "top" }}>
          <circle cx={20} cy={20} r={17} fill="#27ae80" />
        </svg>{" "}
        end
      </Text>
      <Text key="row1" style={{ position: "absolute", left: 12, top: 82, width: 220, height: 150 }}>
        Words{" "}
        <View style={{ display: "inline-block", width: 80, padding: 4, background: "#d6eaf8" }}>
          <Text style={{ fontSize: 14, lineHeight: "20px" }}>two lines inside</Text>
        </View>{" "}
        and more words wrap.
      </Text>
      <Text
        key="row2"
        style={{ position: "absolute", left: 260, top: 82, width: 248, height: 150 }}
      >
        A
        <Span style={{ verticalAlign: "8px" }}>
          <View
            style={{
              display: "inline-flex",
              padding: 4,
              gap: 4,
              alignItems: "center",
              background: "#ffe7ad",
            }}
          >
            <View style={{ width: 14, height: 14, borderRadius: 7, background: "#27ae80" }} />
            <Text style={{ fontSize: 14 }}>OK</Text>
          </View>
        </Span>
        B{" "}
        <View
          style={{
            display: "inline-grid",
            gridTemplateColumns: "16px 16px",
            gap: 3,
            padding: 3,
            verticalAlign: "bottom",
            background: "#d6eaf8",
          }}
        >
          <View style={{ height: 16, background: "#2b71b8" }} />
          <View style={{ height: 16, background: "#27ae80" }} />
        </View>{" "}
        end
      </Text>
    </View>
  );
}
