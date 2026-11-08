export const composition = { width: 640, height: 336, fps: 1, duration: 1 };
export const controls = {
  resources: { CJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
const kinds = ["solid", "double", "dotted", "dashed", "wavy"];
export default function Scene() {
  return (
    <View style={{ width: 640, height: 336, background: "white" }}>
      {kinds.map((kind, row) =>
        ["auto", "12px"].map((offset, column) => (
          <Text
            key={`${kind}-${column}`}
            style={{
              position: "absolute",
              left: 16 + column * 320,
              top: row * 64,
              width: 300,
              height: 64,
              fontFamily: "CJK",
              fontSize: 32,
              fontWeight: 500,
              lineHeight: "64px",
              whiteSpace: "pre",
              color: "#202030",
              // Isolate decoration pixels from differences in glyph antialiasing. The
              // glyph outlines still determine automatic ink-skipping gaps.
              WebkitTextFillColor: "transparent",
              textDecoration: `underline ${kind} 2px #C04060`,
              textUnderlineOffset: offset,
            }}
          >
            强调随行，gyp
          </Text>
        )),
      )}
    </View>
  );
}
