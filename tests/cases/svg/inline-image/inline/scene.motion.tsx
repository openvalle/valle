export const composition = { width: 320, height: 160, fps: 1, duration: 1 };
export const controls = {
  resources: {
    vector: resource({ kind: "image", default: "./vector.svg" }),
    raster: resource({ kind: "image", default: "./raster.png" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 320, height: 160, backgroundColor: "white" }}>
      <svg width="320" height="160">
        <defs>
          <clipPath id="crop">
            <rect x="210" y="20" width="80" height="70" />
          </clipPath>
        </defs>
        <image href="resource://vector" x="10" y="10" width="80" height="100" />
        <image
          href="resource://raster"
          x="110"
          y="10"
          width="80"
          height="100"
          preserveAspectRatio="none"
          imageRendering="pixelated"
        />
        <image
          href="resource://vector"
          x="200"
          y="10"
          width="100"
          height="100"
          preserveAspectRatio="xMaxYMin slice"
          clipPath="url(#crop)"
        />
        <g transform="translate(20 120) scale(2)">
          <image
            href="resource://raster"
            width="40"
            height="10"
            preserveAspectRatio="none"
            imageRendering="pixelated"
          />
        </g>
      </svg>
    </View>
  );
}
