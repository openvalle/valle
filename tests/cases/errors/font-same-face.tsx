// error: errors/font-same-face.tsx: resource brand: https://fonts.gstatic.com/s/notosans/v42/o-0IIpQlx3QUlC5A4PNb4j5Ba_2c7A.ttf: the same weight, style and width as https://fonts.gstatic.com/s/notosans/v42/o-0IIpQlx3QUlC5A4PNb4j5Ba_2c7A.ttf
export const controls = {
  resources: {
    brand: resource({
      kind: "font",
      default: [
        "https://fonts.gstatic.com/s/notosans/v42/o-0IIpQlx3QUlC5A4PNb4j5Ba_2c7A.ttf",
        "https://fonts.gstatic.com/s/notosans/v42/o-0IIpQlx3QUlC5A4PNb4j5Ba_2c7A.ttf",
      ],
    }),
  },
};
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// Two files of a family with the same face: CSS would take one and hide the other.
export default function Scene() {
  return <Text style={{ fontFamily: "brand" }}>A</Text>;
}
