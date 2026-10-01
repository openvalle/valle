export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
const DOTS = Array.from({ length: 64 }, (_, i) => ({
  id: `d${i}`, i, x: 4 + (i % 8) * 8, y: 4 + Math.floor(i / 8) * 14
}));
const MARK = path("M 0 0 L 20 0 L 20 4 Z");

export default function MaskSibling(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <Path key="marker" d={MARK} fill="#ff0000"
      style={{ opacity: bounds("d20").width / 4 }} />
    <Mask key="mask" rect={rect(0, 0, 40, 40)} mode="alpha"
      style={{ position: "absolute", left: 100, top: 20, width: 40, height: 40 }}>
      <View key="mask-content" style={{ width: 40, height: 40, backgroundColor: "#2140ff" }} />
      <MaskSource key="mask-source">
        <Circle key="mask-hole" cx={20} cy={20} r={16} fill="#ffffff" />
      </MaskSource>
    </Mask>
    <Clip key="clip" path={MARK}
      style={{ position: "absolute", left: 180, top: 20, width: 40, height: 40 }}>
      <View key="clip-content" style={{ width: 40, height: 40, backgroundColor: "#ff4a24" }} />
    </Clip>
    <Transition key="transition" kind="circleOpen" progress={ctx.progress}
      style={{ position: "absolute", left: 240, top: 20, width: 40, height: 40 }}>
      <View key="from" style={{ width: 40, height: 40, backgroundColor: "#2140ff" }} />
      <View key="to" style={{ width: 40, height: 40, backgroundColor: "#ff4a24" }} />
    </Transition>
    <Group key="backdrop" className="[mix-blend-mode:screen]"
      style={{ position: "absolute", left: 300, top: 20, width: 40, height: 40 }}>
      <View key="backdrop-content"
        style={{ width: 40, height: 40, backgroundColor: "#406020" }} />
    </Group>
    <Text key="label" split="char"
      perUnit={{ opacity: clamp(ctx.seconds - ctx.unit.index * 0.1, 0, 1) }}
      style={{ position: "absolute", left: 380, top: 20, width: 100, height: 40,
        fontSize: 16, color: "#ffffff" }}>ABC</Text>
    {DOTS.map((dot) => {
      return <View key={dot.id} className="absolute" visible={ctx.seconds * 100 >= dot.i}
        style={{ left: dot.x, top: dot.y, width: 4, height: 4, backgroundColor: "#ffffff",
          opacity: clamp(ctx.seconds * 100 - dot.i, 0, 1) }} />;
    })}
  </Scene>;
}
