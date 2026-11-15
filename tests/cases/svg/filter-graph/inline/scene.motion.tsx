export const composition = { width: 480, height: 360, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 480, height: 360, background: "white" }}>
      <svg width="480" height="360" viewBox="0 0 480 360">
        <defs>
          <filter id="f0" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feGaussianBlur in="SourceAlpha" stdDeviation={2 + seconds} result="blur" />
            <feOffset in="blur" dx="5" dy="6" result="moved" />
            <feFlood floodColor="#1451a0" floodOpacity="0.7" />
            <feComposite in2="moved" operator="in" />
            <feMerge>
              <feMergeNode />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
          <filter id="f1" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feColorMatrix type="saturate" values="0" />
          </filter>
          <filter id="f2" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feOffset dx="18" result="moved" />
            <feBlend
              in="SourceGraphic"
              in2="moved"
              mode="multiply"
              colorInterpolationFilters="sRGB"
            />
          </filter>
          <filter id="f3" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feOffset dx="18" result="moved" />
            <feComposite
              in="SourceGraphic"
              in2="moved"
              operator="arithmetic"
              k2="0.6"
              k3="0.6"
              k4="-0.1"
            />
          </filter>
          <filter id="f4" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feMorphology operator="dilate" radius="2 3" />
          </filter>
          <filter id="f5" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feFlood floodColor="rgb(75% 25% 50%)" result="map" />
            <feDisplacementMap
              in2="map"
              in="SourceGraphic"
              scale="16"
              xChannelSelector="R"
              yChannelSelector="G"
            />
          </filter>
          <filter id="f6" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feDropShadow dx="4" dy="5" stdDeviation="2" floodColor="#751a9e" floodOpacity="0.65" />
          </filter>
          <filter id="f7" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feComponentTransfer>
              <feFuncR type="gamma" exponent="0.7" />
              <feFuncG type="table" tableValues="0.1 0.7 1" />
              <feFuncB type="discrete" tableValues="0.2 0.7" />
              <feFuncA type="linear" slope="0.75" />
            </feComponentTransfer>
          </filter>
          <filter id="f8" primitiveUnits="objectBoundingBox">
            <feFlood
              x="25%"
              y="25%"
              width="50%"
              height="50%"
              floodColor="green"
              floodOpacity="0.75"
            />
          </filter>
          <filter id="f9" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feFlood floodColor="#888" result="gray" />
            <feBlend in="SourceGraphic" in2="gray" mode="multiply" />
          </filter>
          <filter id="f10" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feFlood floodColor="#21a190" floodOpacity="0.7" />
          </filter>
          <filter id="f11" filterUnits="userSpaceOnUse" x="0" y="0" width="120" height="110">
            <feGaussianBlur stdDeviation="1 3" />
            <feColorMatrix type="hueRotate" values="45" />
          </filter>
        </defs>
        <g transform="translate(0 0)">
          <g filter="url(#f0)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(120 0)">
          <g filter="url(#f1)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(240 0)">
          <g filter="url(#f2)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(360 0)">
          <g filter="url(#f3)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(0 120)">
          <g filter="url(#f4)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(120 120)">
          <g filter="url(#f5)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(240 120)">
          <g filter="url(#f6)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(360 120)">
          <g filter="url(#f7)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(0 240)">
          <g filter="url(#f8)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(120 240)">
          <g filter="url(#f9)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
        <g transform="translate(240 240)">
          <g filter="url(#f10)">
            <rect x="20" y="20" width="50" height="40" fill="none" />
          </g>
        </g>
        <g transform="translate(360 240)">
          <g filter="url(#f11)">
            <rect x="20" y="20" width="50" height="40" rx="3" fill="#2365c7" />
            <circle cx="65" cy="50" r="20" fill="#ed852b" fillOpacity="0.65" />
          </g>
        </g>
      </svg>
    </View>
  );
}
