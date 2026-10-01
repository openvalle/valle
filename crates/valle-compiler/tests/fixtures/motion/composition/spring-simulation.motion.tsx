export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

const SPRING = simulate({
  dt: 1 / 240,
  duration: 2,
  init: () => ({ x: 100, v: 0 }),
  step: (state, t, dt) => {
    const a = (300 - state.x) * 40 - state.v * 4;
    const v = state.v + a * dt;
    return { x: state.x + v * dt, v };
  },
});

export default function Simulated(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <View key="dot" className="absolute" style={{
        left: SPRING.at(ctx.seconds).x,
        top: 170,
        width: 20,
        height: 20,
        backgroundColor: "#ffffff",
      }} />
    </Scene>
  );
}
