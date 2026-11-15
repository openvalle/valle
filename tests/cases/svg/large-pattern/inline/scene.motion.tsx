export const composition = { width: 480, height: 220, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 480, height: 220, backgroundColor: "white" }}>
      <svg width="480" height="220">
        <defs>
          <pattern id="large" width="8192" height="8192" patternUnits="userSpaceOnUse">
            <rect x="12" y="12" width="1" height="176" fill="#17365d" />
            <rect x="24" y="12" width="1" height="176" fill="#17365d" />
            <rect x="36" y="12" width="1" height="176" fill="#17365d" />
            <rect x="48" y="12" width="1" height="176" fill="#17365d" />
            <rect x="60" y="12" width="1" height="176" fill="#17365d" />
            <rect x="72" y="12" width="1" height="176" fill="#17365d" />
            <rect x="84" y="12" width="1" height="176" fill="#17365d" />
            <rect x="96" y="12" width="1" height="176" fill="#17365d" />
            <rect x="108" y="12" width="1" height="176" fill="#17365d" />
            <rect x="120" y="12" width="1" height="176" fill="#17365d" />
            <rect x="12" y="100" width="109" height="1" fill="#d06030" />
          </pattern>
          <pattern
            id="repeat"
            x="-12.5"
            y="-2.5"
            width="8192"
            height="10.5"
            patternUnits="userSpaceOnUse"
          >
            <rect width="8192" height="10.5" fill="#2878b8" />
          </pattern>
          <pattern
            id="turned"
            width="8192"
            height="8192"
            patternUnits="userSpaceOnUse"
            patternTransform="translate(320 20) rotate(12)"
          >
            <rect x="12" y="12" width="1" height="176" fill="#17365d" />
            <rect x="24" y="12" width="1" height="176" fill="#17365d" />
            <rect x="36" y="12" width="1" height="176" fill="#17365d" />
            <rect x="48" y="12" width="1" height="176" fill="#17365d" />
            <rect x="60" y="12" width="1" height="176" fill="#17365d" />
            <rect x="72" y="12" width="1" height="176" fill="#17365d" />
            <rect x="84" y="12" width="1" height="176" fill="#17365d" />
            <rect x="96" y="12" width="1" height="176" fill="#17365d" />
            <rect x="108" y="12" width="1" height="176" fill="#17365d" />
            <rect x="120" y="12" width="1" height="176" fill="#17365d" />
          </pattern>
        </defs>
        <rect x="10" y="10" width="130" height="190" fill="url(#large)" />
        <rect x="170" y="10" width="130" height="190" fill="url(#repeat)" fillOpacity="0.5" />
        <rect x="330" y="10" width="130" height="190" fill="url(#turned)" />
      </svg>
    </View>
  );
}
