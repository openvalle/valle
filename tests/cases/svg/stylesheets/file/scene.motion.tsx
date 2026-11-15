export const composition = { width: 300, height: 110, fps: 1, duration: 1 };
export const controls = {
  resources: { art: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return <Image src="resource://art" style={{ width: 300, height: 110 }} />;
}
