export const composition = { width: 1920, height: 1080, fps: 60, duration: 5 };

// Easings.
const IN_OUT_CUBIC = "cubic-bezier(0.65, 0, 0.35, 1)";
const OUT_CUBIC = "cubic-bezier(0.33, 1, 0.68, 1)";

export default function Launch() {
  const { seconds } = useCurrentTime();
  return (
    <View className="grid size-full place-items-center bg-linear-to-br from-indigo-600 via-fuchsia-500 to-orange-400">
      <Text
        split="char"
        className="text-9xl font-black tracking-tight text-white"
        perUnit={(unit) => {
          const enter = interpolate(seconds, [unit.index * 0.04, 0.8 + unit.index * 0.04], [0, 1], {
            easing: OUT_CUBIC,
          });
          const leave = interpolate(
            seconds,
            [3.1 + unit.index * 0.025, 3.8 + unit.index * 0.025],
            [0, 1],
            { easing: IN_OUT_CUBIC },
          );
          const shown = clamp(enter - leave, 0, 1);
          return {
            opacity: shown,
            filter: `blur(${16 * (1 - shown)}px)`,
            translate: `0px ${40 * (1 - shown)}px`,
          };
        }}
      >
        Hello, Valle!
      </Text>
    </View>
  );
}
