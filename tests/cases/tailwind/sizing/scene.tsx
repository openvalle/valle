export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Sizing and placing classes: widths in fractions, px, arbitrary values and calc(); insets,
// offsets in fractions and negative ones, and z-index in a stacking context; aspect ratios;
// clipping; hidden, invisible and fit-content children; content-box sizing; min and max widths,
// one of the container scale; an arbitrary type and arbitrary properties.
export default function Scene() {
  const red = { background: "#e53935" };
  const blue = { background: "#1e88e5" };
  const green = { background: "#43a047" };
  return (
    <View
      className="flex p-2 gap-2"
      style={{ flexWrap: "wrap", alignItems: "flex-start", background: "#eceff1" }}
    >
      <View key="fractions" className="w-20" style={blue}>
        <View key="half" className="w-1/2 h-3" style={red} />
        <View key="three-quarters" className="w-3/4 h-px" style={green} />
        <View key="calc" className="w-[calc(100%-1rem)] h-2" style={red} />
        <View key="full" className="w-full h-[5px]" style={green} />
      </View>
      <View key="position" className="relative size-16" style={{ ...blue, zIndex: 0 }}>
        <View key="inset" className="absolute inset-2" style={red} />
        <View key="half" className="absolute top-1 left-1/2 size-3" style={green} />
        <View key="negative" className="absolute -right-1 -bottom-1 size-4 z-10" style={green} />
        <View className="absolute inset-x-0 bottom-0 h-3 -z-10" style={{ background: "#fdd835" }} />
      </View>
      <View key="video" className="w-12 aspect-video" style={blue} />
      <View key="photo" className="w-12 aspect-3/2" style={red} />
      <View className="w-10 aspect-square" style={green} />
      <View className="size-10 overflow-hidden" style={blue}>
        <View className="w-16 h-4 ml-6" style={red} />
      </View>
      <View className="flex w-20 h-6 p-1" style={blue}>
        <View className="hidden w-4" style={red} />
        <View className="invisible w-4" style={red} />
        <View className="w-fit" style={green}>
          <View className="w-5 h-3" />
        </View>
      </View>
      <View key="content-box" className="box-content size-8 p-2" style={blue}>
        <View key="child" className="size-full" style={red} />
      </View>
      <View key="minimum" className="min-w-12 w-2 h-4" style={blue} />
      <View key="maximum" className="max-w-8 w-20 h-4" style={red} />
      <View className="w-xs max-w-16 h-4" style={green} />
      <View className="w-[length:3rem] h-[11px]" style={blue} />
      <View className="[width:40px] [height:12px]" style={red} />
    </View>
  );
}
