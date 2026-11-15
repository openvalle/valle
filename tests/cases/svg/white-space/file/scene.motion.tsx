export const composition = { width: 320, height: 180, fps: 1, duration: 1 };
export const controls = {
  resources: { picture: resource({ kind: "image", default: "./source.svg" }) },
};
export default function Scene() {
  return <Image src="resource://picture" style={{ width: 320, height: 180 }} />;
}
