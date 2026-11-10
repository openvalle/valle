export const composition = { width: 320, height: 180, fps: 30, duration: 2 };
export const controls = {
  props: { label: string({ default: "你好😀" }) },
  resources: { image: resource({ kind: "image", default: "./image.png" }) },
};
export default function Scene({ label }) {
  return <Text>{label}</Text>;
}
