export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Flex classes: directions, reversed and wrapped; justifying and aligning, the lines too; items
// that grow, shrink and start from a basis, a fraction or a number; the flex shorthands; items
// out of order; and items that align themselves.
export default function Scene() {
  const red = { background: "#e53935" };
  const blue = { background: "#1e88e5" };
  const green = { background: "#43a047" };
  const yellow = { background: "#fdd835" };
  return (
    <View className="flex flex-wrap items-start gap-2 p-2" style={{ background: "#eceff1" }}>
      <View key="between" className="flex items-center justify-between w-24 h-10 p-1" style={blue}>
        <View className="size-3" style={red} />
        <View key="green" className="w-3 h-6" style={green} />
        <View key="yellow" className="w-3 h-2" style={yellow} />
      </View>
      <View key="column" className="flex flex-col items-end gap-1 w-12 p-1" style={blue}>
        <View key="short" className="w-4 h-2" style={red} />
        <View key="wide" className="w-7 h-2" style={green} />
        <View key="start" className="self-start w-3 h-2" style={yellow} />
      </View>
      <View
        key="reverse"
        className="flex flex-row-reverse justify-center gap-1 w-20 p-1"
        style={blue}
      >
        <View className="size-3" style={red} />
        <View className="size-3" style={green} />
      </View>
      <View className="flex flex-col-reverse justify-evenly h-14 w-8 px-1" style={blue}>
        <View className="h-2" style={red} />
        <View className="h-2" style={green} />
        <View className="h-2" style={yellow} />
      </View>
      <View className="flex flex-wrap content-between gap-x-1 w-16 h-14 p-1" style={blue}>
        {[red, green, yellow, red, green].map((s, i) => (
          <View key={i} className="w-4 h-3" style={s} />
        ))}
      </View>
      <View className="flex flex-wrap-reverse justify-around w-16 p-1" style={blue}>
        {[red, green, yellow].map((s, i) => (
          <View key={i} className="w-6 h-3" style={s} />
        ))}
      </View>
      <View className="flex w-40 h-4" style={blue}>
        <View className="flex-1" style={red} />
        <View className="grow-[2]" style={green} />
        <View className="flex-none w-3" style={yellow} />
        <View className="basis-1/4" style={green} />
        <View className="shrink-0 basis-6" style={red} />
      </View>
      <View className="flex w-24 h-4" style={blue}>
        <View className="flex-auto w-4" style={red} />
        <View className="flex-initial w-12" style={green} />
        <View className="flex-[2_2_0%]" style={yellow} />
      </View>
      <View className="flex gap-1 p-1" style={blue}>
        <View className="order-last w-2 h-4" style={red} />
        <View className="order-2 w-4 h-4" style={green} />
        <View className="-order-1 w-6 h-4" style={yellow} />
        <View className="order-first w-3 h-4" style={red} />
      </View>
      <View className="flex items-stretch gap-1 h-12 p-1" style={blue}>
        <View className="w-3" style={red} />
        <View className="self-center w-3 h-3" style={green} />
        <View className="self-end w-3 h-5" style={yellow} />
        <View className="self-auto w-3" style={red} />
      </View>
    </View>
  );
}
