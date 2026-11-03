// error: errors/flip-layout-id.tsx:12:10: TypeError: <View> style layoutTransition needs a layoutId
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

const layouts = defineLayoutStates({
  small: { a: { x: 0, y: 0, width: 16, height: 16 } },
  large: { a: { x: 0, y: 0, width: 64, height: 64 } },
});

export default function Scene() {
  const { progress } = useCurrentTime();
  const f = flip(layouts, "small", "large", progress);
  return <View style={{ background: "red", layoutTransition: f }} />;
}
