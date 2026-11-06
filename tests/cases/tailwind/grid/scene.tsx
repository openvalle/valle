export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Grid classes: columns and rows of equal fractions or arbitrary tracks, implicit tracks, items
// spanning and placed by line, negative lines too, dense and column flow; placing and justifying
// the items and the tracks; and items that place themselves.
export default function Scene() {
  const red = { background: "#e53935" };
  const blue = { background: "#1e88e5" };
  const green = { background: "#43a047" };
  const yellow = { background: "#fdd835" };
  return (
    <View className="flex flex-wrap items-start gap-2 p-2" style={{ background: "#eceff1" }}>
      <View key="tracks" className="grid grid-cols-3 gap-1 w-20 p-1" style={blue}>
        <View className="col-span-2 h-3" style={red} />
        <View className="row-span-2" style={green} />
        <View className="h-3" style={yellow} />
        <View className="h-3" style={red} />
        <View key="full" className="col-span-full h-2" style={yellow} />
      </View>
      <View key="placement" className="grid grid-cols-4 gap-1 w-20 p-1" style={blue}>
        <View className="col-start-2 col-end-4 h-3" style={red} />
        <View className="-col-start-2 h-3" style={green} />
        <View key="full" className="row-start-2 col-start-1 -col-end-1 h-2" style={yellow} />
      </View>
      <View
        key="columns"
        className="grid grid-flow-col grid-rows-2 auto-cols-[10px] gap-1 p-1"
        style={blue}
      >
        {[red, green, yellow, red, green].map((s, i) => (
          <View key={i} className="h-3" style={s} />
        ))}
      </View>
      <View
        key="dense"
        className="grid grid-cols-[repeat(4,8px)] grid-flow-row-dense gap-1 p-1"
        style={blue}
      >
        <View className="h-3" style={red} />
        <View className="col-span-3 h-3" style={green} />
        <View className="col-span-2 h-3" style={yellow} />
        <View className="h-3" style={red} />
      </View>
      <View key="center" className="grid place-items-center size-12" style={blue}>
        <View key="child" className="size-4" style={red} />
      </View>
      <View
        className="grid grid-cols-2 justify-items-end items-start gap-1 w-16 h-12 p-1"
        style={blue}
      >
        <View className="size-3" style={red} />
        <View className="place-self-center size-3" style={green} />
        <View className="justify-self-start size-3" style={yellow} />
        <View className="self-end justify-self-center size-3" style={red} />
      </View>
      <View className="grid grid-rows-[6px_1fr] auto-rows-fr gap-1 w-8 h-16 p-1" style={blue}>
        <View style={red} />
        <View style={green} />
        <View style={yellow} />
      </View>
      <View
        className="grid grid-cols-[repeat(2,10px)] auto-rows-[8px] place-content-center gap-1 size-16"
        style={blue}
      >
        {[red, green, yellow].map((s, i) => (
          <View key={i} style={s} />
        ))}
      </View>
      <View
        className="grid grid-cols-3 justify-between content-end gap-y-1 w-24 h-14 p-1"
        style={blue}
      >
        {[red, green, yellow, red].map((s, i) => (
          <View key={i} className="w-4 h-3" style={s} />
        ))}
      </View>
      <View className="grid grid-cols-2 auto-rows-min gap-1 w-14 h-12 p-1" style={blue}>
        <View className="h-2" style={red} />
        <View className="row-span-2 h-6" style={green} />
        <View className="row-end-3 h-2" style={yellow} />
      </View>
    </View>
  );
}
