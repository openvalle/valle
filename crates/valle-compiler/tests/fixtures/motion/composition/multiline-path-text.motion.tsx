export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function PathLines(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Text key="arc" path={arc(point(320, 340), 260, deg(200), deg(340))}
        style={{ fontSize: 28, color: "#ffffff", whiteSpace: "pre-line", lineHeight: 1.3 }}>
        {"FIRST LINE ON A CURVE\nSECOND LINE BELOW IT"}
      </Text>
    </Scene>
  );
}
