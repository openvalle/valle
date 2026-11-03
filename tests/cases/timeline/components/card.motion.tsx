export const composition = { width: 80, height: 60, fps: 4, duration: 2 };

export const controls = {
  props: {
    label: string({ default: "card" }),
    color: color({ default: "#f97316" }),
  },
  resources: {
    picture: resource({ kind: "image", default: "../../resources/tile.png" }),
  },
};

// A card that grows across its 2 s: its label, its picture, and a bar of its progress.
export default function Card({ label, color }) {
  const { progress } = useCurrentTime();
  return (
    <View
      style={{
        width: 80,
        height: 60,
        background: color,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 4,
      }}
    >
      <Image src="resource://picture" style={{ width: 24, height: 24 }} />
      <Text style={{ fontSize: 10, color: "white" }}>{label}</Text>
      <View style={{ width: 70 * progress, height: 4, background: "white" }} />
    </View>
  );
}
