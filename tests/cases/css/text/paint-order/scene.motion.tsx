export const composition = { width: 640, height: 160, fps: 1, duration: 1 };
export const controls = {
  resources: { StrokeCJK: resource({ kind: "font", default: "/tests/resources/NotoSansSC.ttf" }) },
};
export default function Scene() {
  return (
    <View
      style={{
        width: 640,
        height: 160,
        position: "relative",
        background: "#607a90",
        paintOrder: "stroke fill",
      }}
    >
      <Text
        key="text"
        style={{
          position: "absolute",
          left: 20,
          top: 10,
          width: 600,
          height: 140,
          fontFamily: "StrokeCJK",
          fontSize: 48,
          fontWeight: 500,
          lineHeight: "70px",
          whiteSpace: "pre",
          color: "white",
          WebkitTextStroke: "2px #14141a",
        }}
      >
        {"第 0001 句：字幕测试\nCaption 012 ABC / Valle"}
      </Text>
    </View>
  );
}
