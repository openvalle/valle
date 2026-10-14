export const composition = { width: 640, height: 360, fps: 60, duration: 10 };

const hex = (v) => v.toString(16).padStart(2, "0");
// 512 flat fills: rows 0-7 sweep red (green 189, blue 248), rows 8-15 sweep green (red 56, blue 248).
// Working colours are linear Rec.2020; an 8-bit stage anywhere before the output shows here.
const CELLS = Array.from({ length: 512 }, (_, i) => {
  const v = i % 256;
  const color = i < 256 ? `#${hex(v)}bdf8` : `#38${hex(v)}f8`;
  return { id: `c${i}`, x: (i % 32) * 20, y: Math.floor(i / 32) * 22.5, color };
});

export default function FlatColors() {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#e2e8f0" }}>
      {CELLS.map(c => <View key={c.id} style={{ position: "absolute", left: c.x, top: c.y, width: 20, height: 22.5, backgroundColor: c.color }} />)}
    </Scene>
  );
}
