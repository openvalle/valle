export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function CssOklab(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="g" className="absolute" style={{ left: 0, top: 0, width: 640, height: 360,
        backgroundImage: "linear-gradient(90deg in oklab, #ff0000, #0000ff)" }} />
    </Scene>
  );
}
