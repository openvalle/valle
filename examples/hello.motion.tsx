export default function Hello(ctx) {
  const opacity = interpolate(ctx.hold.progress, [0, 0.6], [0, 1]);

  return (
    <Scene className="relative h-full w-full flex flex-col items-center justify-center"
      style={{ backgroundColor: "#102030" }}>
      <Text style={{ fontSize: 48, fontWeight: 700, color: "#ffffff", opacity }}>
        Hello, Valle!
      </Text>
      <Text style={{ marginTop: 16, fontSize: 20, color: "#94a3b8", opacity }}>
        Create with code. Bring it to life.
      </Text>
    </Scene>
  );
}
