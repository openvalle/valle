// error: errors/font-image.tsx:10:10: <Image> src resource://display: display is a font, not an image
export const controls = {
  resources: {
    display: resource({ kind: "font", default: "../resources/Bungee-Regular.ttf" }),
  },
};
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <Image src="resource://display" style={{ width: 32, height: 32 }} />;
}
