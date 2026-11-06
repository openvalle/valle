export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Spacing classes: padding on each side, in steps of the scale, a half step, px and an arbitrary
// value; margins, negative and auto; the space between children, reversed and negative, under
// the children's own margins; gaps in a grid; and the style over a class, an important class
// over the style.
export default function Scene() {
  const red = { background: "#e53935" };
  const blue = { background: "#1e88e5" };
  const tiles = (n: number, className = "size-4") =>
    Array.from({ length: n }, (_, i) => <View key={i} className={className} style={red} />);
  return (
    <View className="flex p-2 gap-2" style={{ flexWrap: "wrap", background: "#eceff1" }}>
      <View key="sides" className="pt-1 pr-2 pb-3 pl-4" style={blue}>
        {tiles(1, "size-6")}
      </View>
      <View key="fractional" className="px-1.5 py-[3px]" style={blue}>
        {tiles(1, "size-6")}
      </View>
      <View key="margins" className="w-12 p-px" style={blue}>
        <View key="negative" className="w-6 h-[18px] m-1 -mt-2 ml-auto" style={red} />
      </View>
      <View key="auto" className="flex w-24 p-1" style={blue}>
        {tiles(1, "size-4 mx-auto")}
      </View>
      <View key="space" className="flex space-x-2 p-1" style={blue}>
        {tiles(3)}
      </View>
      <View
        className="flex space-x-3 space-x-reverse p-1"
        style={{ ...blue, flexDirection: "row-reverse" }}
      >
        {tiles(3)}
      </View>
      <View className="flex -space-x-2 p-1" style={blue}>
        {tiles(2)}
        <View className="size-4 ms-1" style={{ background: "#43a047" }} />
        {tiles(1)}
      </View>
      <View
        key="vertical"
        className="flex space-y-1 p-1"
        style={{ ...blue, flexDirection: "column" }}
      >
        {tiles(3, "w-6 h-2")}
      </View>
      <View
        className="grid gap-x-1 gap-y-2 p-1"
        style={{ ...blue, gridTemplateColumns: "repeat(3, 12px)" }}
      >
        {tiles(6, "h-3")}
      </View>
      <View key="style" className="p-4 m-0" style={{ ...blue, padding: 2 }}>
        {tiles(1)}
      </View>
      <View key="important" className="p-4!" style={{ ...blue, padding: 2 }}>
        {tiles(1)}
      </View>
    </View>
  );
}
