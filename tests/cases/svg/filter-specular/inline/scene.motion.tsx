export const composition = { width: 340, height: 130, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 340, height: 130, background: "white" }}>
      <svg width="340" height="130" viewBox="0 0 340 130">
        <rect width="340" height="130" fill="#111827" />
        <defs>
          <filter id="f0" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feSpecularLighting
              surfaceScale="6"
              specularConstant="0.8"
              specularExponent="12"
              lightingColor="#bae6fd"
            >
              <feDistantLight azimuth="35" elevation={65 + seconds * 8} />
            </feSpecularLighting>
          </filter>
          <filter id="f1" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feSpecularLighting
              surfaceScale="6"
              specularConstant="0.8"
              specularExponent="12"
              lightingColor="#bae6fd"
            >
              <fePointLight x="25" y="30" z="50" />
            </feSpecularLighting>
          </filter>
          <filter id="f2" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feSpecularLighting
              surfaceScale="6"
              specularConstant="0.8"
              specularExponent="12"
              lightingColor="#bae6fd"
            >
              <feSpotLight
                x="50"
                y="30"
                z="70"
                pointsAtX="60"
                pointsAtY="60"
                pointsAtZ="0"
                specularExponent="3"
              />
            </feSpecularLighting>
          </filter>
        </defs>
        <g transform="translate(5 15)">
          <rect width="100" height="100" filter="url(#f0)" />
        </g>
        <g transform="translate(117 15)">
          <rect width="100" height="100" filter="url(#f1)" />
        </g>
        <g transform="translate(229 15)">
          <rect width="100" height="100" filter="url(#f2)" />
        </g>
      </svg>
    </View>
  );
}
