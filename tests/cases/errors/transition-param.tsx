// error: errors/transition-param.tsx:7:5: <Transition> wipeLeft has no param radius; it takes softness
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// A built-in kind takes its own params only.
export default function Scene() {
  return (
    <Transition kind="wipeLeft" params={{ radius: 2 }} style={{ width: 8, height: 8 }}>
      <View />
      <View />
    </Transition>
  );
}
