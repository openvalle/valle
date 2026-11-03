// error: errors/font-image.tsx:13:10: <Image> src resource://display: display is a font, not an image
export const controls = {
  resources: {
    display: resource({
      kind: "font",
      default: "https://fonts.gstatic.com/s/notoserif/v33/ga6Iaw1J5X9T9RW6j9bNTFAcaRi_bMQ.ttf",
    }),
  },
};
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return <Image src="resource://display" style={{ width: 32, height: 32 }} />;
}
