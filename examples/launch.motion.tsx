export const composition = { width: 1920, height: 1080, fps: 60, duration: 5 };

export default function Launch() {
  const { seconds } = useCurrentTime();
  return (
    <Scene className="grid size-full place-items-center bg-linear-to-br from-indigo-600 via-fuchsia-500 to-orange-400">
      <Text split="char" className="text-9xl font-black tracking-tight text-white" perUnit={(unit) => {
        const enter = interpolate(seconds, [unit.index * 0.04, 0.8 + unit.index * 0.04], [0, 1], { easing: "easeOutCubic" });
        const leave = interpolate(seconds, [3.1 + unit.index * 0.025, 3.8 + unit.index * 0.025], [0, 1], { easing: "easeInOutCubic" });
        const shown = clamp(enter - leave, 0, 1);
        return { opacity: shown, blur: 16 * (1 - shown), translate: point(0, 40 * (1 - shown)) };
      }}>
        Hello, Valle!
      </Text>
    </Scene>
  );
}
