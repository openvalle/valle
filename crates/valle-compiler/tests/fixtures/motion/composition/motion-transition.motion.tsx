export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function TransitionView(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Transition key="t" kind="circleOpen" progress={ctx.progress}
        style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }}>
        <View key="from" style={{ width: 640, height: 360, backgroundColor: "#2140ff" }} />
        <View key="to" style={{ width: 640, height: 360, backgroundColor: "#ff4a24" }} />
      </Transition>
    </Scene>
  );
}
