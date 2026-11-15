export const composition = { width: 300, height: 100, fps: 1, duration: 1 };
export const controls = {
  resources: { icon: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return <Image src="resource://icon" style={{ width: 300, height: 100 }} />;
}
