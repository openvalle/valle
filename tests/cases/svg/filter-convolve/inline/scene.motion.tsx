export const composition = { width: 450, height: 130, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 450, height: 130, background: "white" }}>
      <svg width="450" height="130" viewBox="0 0 450 130">
        <defs>
          <filter id="f0" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feConvolveMatrix order="3" kernelMatrix="0 -1 0 -1 5 -1 0 -1 0" />
          </filter>
          <filter id="f1" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feConvolveMatrix order="3 1" kernelMatrix="1 0 0" targetX="0" edgeMode="wrap" />
          </filter>
          <filter id="f2" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feConvolveMatrix
              order="3"
              kernelMatrix="1 1 1 1 1 1 1 1 1"
              kernelUnitLength="2"
              edgeMode="none"
            />
          </filter>
          <filter id="f3" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feConvolveMatrix
              order="3"
              kernelMatrix="-1 0 0 0 0 0 0 0 1"
              bias={0.5 + seconds * 0.1}
              preserveAlpha="true"
            />
          </filter>
        </defs>
        <g transform="translate(5 15)" filter="url(#f0)">
          <rect width="100" height="100" fill="#2563eb" fillOpacity="0.5" />
          <rect x="0" y="0" width="12" height="100" fill="#f97316" />
          <rect x="25" y="25" width="55" height="50" fill="#10b981" />
        </g>
        <g transform="translate(117 15)" filter="url(#f1)">
          <rect width="100" height="100" fill="#2563eb" fillOpacity="0.5" />
          <rect x="0" y="0" width="12" height="100" fill="#f97316" />
          <rect x="25" y="25" width="55" height="50" fill="#10b981" />
        </g>
        <g transform="translate(229 15)" filter="url(#f2)">
          <rect width="100" height="100" fill="#2563eb" fillOpacity="0.5" />
          <rect x="0" y="0" width="12" height="100" fill="#f97316" />
          <rect x="25" y="25" width="55" height="50" fill="#10b981" />
        </g>
        <g transform="translate(341 15)" filter="url(#f3)">
          <rect width="100" height="100" fill="#2563eb" fillOpacity="0.5" />
          <rect x="0" y="0" width="12" height="100" fill="#f97316" />
          <rect x="25" y="25" width="55" height="50" fill="#10b981" />
        </g>
      </svg>
    </View>
  );
}
