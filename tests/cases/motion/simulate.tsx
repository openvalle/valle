export const composition = { width: 256, height: 128, fps: 4, duration: 2 };

// A ball dropped onto a floor, losing some speed at each bounce, and a damped pendulum, both
// stepped at 240 Hz once and read at each frame's time.
const ball = simulate({
  dt: 1 / 240,
  duration: 2,
  init: () => ({ y: 0, v: 0 }),
  step: ({ y, v }, t, dt) => {
    const fall = { y: y + v * dt, v: v + 900 * dt };
    return fall.y > 96 ? { y: 96, v: -0.7 * fall.v } : fall;
  },
});
const pendulum = simulate({
  dt: 1 / 240,
  duration: 2,
  init: () => ({ angle: 1, speed: 0 }),
  step: ({ angle, speed }, t, dt) => ({
    angle: angle + speed * dt,
    speed: speed + (-40 * Math.sin(angle) - 0.8 * speed) * dt,
  }),
});

export default function Scene() {
  const { seconds } = useCurrentTime();
  const { y } = ball.at(seconds);
  const { angle } = pendulum.at(seconds);
  return (
    <View style={{ width: 256, height: 128 }}>
      <View
        style={{
          position: "absolute",
          left: 40,
          top: y,
          width: 24,
          height: 24,
          borderRadius: 12,
          background: "#e53935",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 160,
          top: 8,
          width: 4,
          height: 96,
          background: "#1e88e5",
          transformOrigin: "50% 0",
          rotate: `${angle}rad`,
        }}
      />
    </View>
  );
}
