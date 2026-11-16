export const composition = { width: 784, height: 544, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 784, height: 544, background: "white" }}>
      <Image src="resource://picture" style={{ width: 784, height: 544 }} />
    </View>
  );
}
