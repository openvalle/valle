export const composition = { width: 520, height: 140, fps: 4, duration: 2 };
export const controls = {
  resources: {
    vector: resource({ kind: "image", default: "./tile.svg" }),
    raster: resource({ kind: "image", default: "./tile.png" }),
  },
};
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 520, height: 140, background: "white" }}>
      <svg width="520" height="140" viewBox="0 0 520 140">
        <defs>
          <g id="stamp">
            <rect x="3" y="5" width="32" height="20" fill="#2563eb" />
            <rect x="10" y="10" width="15" height="25" fill="#f97316" />
          </g>
          <filter id="local" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="120">
            <feImage href="#stamp" x={15 + seconds * 20} y="18" width="90" height="84" />
          </filter>
          <filter id="vector" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="120">
            <feImage href="resource://vector" x="10" y="5" width="80" height="90" />
          </filter>
          <filter id="raster" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="120">
            <feImage
              href="resource://raster"
              x="10"
              y="5"
              width="80"
              height="90"
              preserveAspectRatio="none"
              imageRendering="pixelated"
            />
          </filter>
          <filter id="masked" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="120">
            <feImage
              href="resource://vector"
              x="10"
              y="5"
              width="80"
              height="90"
              preserveAspectRatio="xMaxYMin slice"
              result="image"
            />
            <feComposite in="SourceGraphic" in2="image" operator="in" />
          </filter>
        </defs>
        <g transform="translate(5 10)">
          <rect width="120" height="120" fill="#db2777" filter="url(#local)" />
        </g>
        <g transform="translate(135 10)">
          <rect width="120" height="120" fill="#db2777" filter="url(#vector)" />
        </g>
        <g transform="translate(265 10)">
          <rect width="120" height="120" fill="#db2777" filter="url(#raster)" />
        </g>
        <g transform="translate(395 10)">
          <rect width="120" height="120" fill="#db2777" filter="url(#masked)" />
        </g>
      </svg>
    </View>
  );
}
