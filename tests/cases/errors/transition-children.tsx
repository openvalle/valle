// error: errors/transition-children.tsx:7:5: TypeError: <Transition> takes two children: what it goes from, then what it goes to
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// It goes from one child to another.
export default function Scene() {
  return (
    <Transition kind="fade">
      <View />
    </Transition>
  );
}
