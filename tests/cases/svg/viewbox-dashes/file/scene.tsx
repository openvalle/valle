export const composition = { width: 340, height: 260, fps: 1, duration: 1 };
export const controls = {
  resources: {
    drawing: resource({ kind: "image", default: "./source.svg" }),
    center: resource({ kind: "image", default: "./meet-center.svg" }),
    slice: resource({ kind: "image", default: "./slice.svg" }),
    stretch: resource({ kind: "image", default: "./stretch.svg" }),
    end: resource({ kind: "image", default: "./meet-end.svg" }),
  },
};

export default function Scene() {
  const variants = [
    { key: "meet-center", src: "resource://center" },
    { key: "slice", src: "resource://slice" },
    { key: "stretch", src: "resource://stretch" },
    { key: "meet-end", src: "resource://end" },
  ];
  return (
    <View style={{ width: 340, height: 260, backgroundColor: "white" }}>
      <Image
        key="drawing"
        src="resource://drawing"
        style={{ position: "absolute", left: 20, top: 20, width: 240, height: 120 }}
      />
      {variants.map((variant, i) => (
        <Image
          key={variant.key}
          src={variant.src}
          style={{
            position: "absolute",
            left: 20 + i * 80,
            top: 180,
            width: 60,
            height: 60,
            backgroundColor: "#eef2f7",
          }}
        />
      ))}
    </View>
  );
}
