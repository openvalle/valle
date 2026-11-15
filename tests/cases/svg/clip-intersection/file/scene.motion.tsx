export const composition = { width: 320, height: 160, fps: 1, duration: 1 };
export const controls = {
  resources: { image: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return <Image src="resource://image" style={{ width: 320, height: 160 }} />;
}
