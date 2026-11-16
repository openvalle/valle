export const composition = { width: 800, height: 560, fps: 4, duration: 2 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
const SHIFTS = [
  [0, 0],
  [4, 2],
  [8, 4],
  [2, 8],
  [6, 6],
  [0, 0],
  [8, 4],
  [4, 2],
];
export default function Scene() {
  const { seconds } = useCurrentTime();
  const [x, y] = SHIFTS[Math.floor(seconds * 4)];
  return (
    <View style={{ width: 800, height: 560, background: "white" }}>
      <Image
        src="resource://picture"
        style={{ position: "absolute", left: x, top: y, width: 784, height: 544 }}
      />
    </View>
  );
}
