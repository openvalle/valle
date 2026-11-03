// error: errors/unit-style.tsx:7:5: style padding: a unit of a split text takes only text properties, opacity, transforms and filter
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// A unit is no box to pad.
export default function Scene() {
  return (
    <Text split="char" perUnit={() => ({ padding: 4 })}>
      ab
    </Text>
  );
}
