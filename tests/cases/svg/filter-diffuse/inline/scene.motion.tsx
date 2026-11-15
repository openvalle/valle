export const composition = { width: 340, height: 130, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 340, height: 130, background: "white" }}>
      <svg width="340" height="130" viewBox="0 0 340 130">
        <defs>
          <linearGradient id="height">
            <stop stopColor="white" stopOpacity="0.1" />
            <stop offset="1" stopColor="white" />
          </linearGradient>
          <filter id="f0" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feDiffuseLighting surfaceScale="6" diffuseConstant="0.8" lightingColor="#ffe5b4">
              <feDistantLight azimuth={35 + seconds * 25} elevation="45" />
            </feDiffuseLighting>
          </filter>
          <filter id="f1" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feDiffuseLighting surfaceScale="6" diffuseConstant="0.8" lightingColor="#ffe5b4">
              <fePointLight x="25" y="30" z="80" />
            </feDiffuseLighting>
          </filter>
          <filter id="f2" x="0" y="0" width="100%" height="100%" colorInterpolationFilters="sRGB">
            <feDiffuseLighting surfaceScale="6" diffuseConstant="0.8" lightingColor="#ffe5b4">
              <feSpotLight
                x="50"
                y="30"
                z="80"
                pointsAtX="60"
                pointsAtY="60"
                pointsAtZ="0"
                specularExponent="3"
                limitingConeAngle="75"
              />
            </feDiffuseLighting>
          </filter>
        </defs>
        <g transform="translate(5 15)">
          <rect width="100" height="100" fill="url(#height)" filter="url(#f0)" />
        </g>
        <g transform="translate(117 15)">
          <rect width="100" height="100" fill="url(#height)" filter="url(#f1)" />
        </g>
        <g transform="translate(229 15)">
          <rect width="100" height="100" fill="url(#height)" filter="url(#f2)" />
        </g>
      </svg>
    </View>
  );
}
