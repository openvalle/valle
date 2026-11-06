export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <mask id="luminance" maskUnits="userSpaceOnUse" x="20" y="20" width="120" height="120">
            <rect x="20" y="20" width="60" height="120" fill="white" />
            <rect x="80" y="20" width="60" height="120" fill="black" />
          </mask>
          <mask
            id="alpha"
            maskType="alpha"
            maskUnits="userSpaceOnUse"
            x="180"
            y="20"
            width="120"
            height="120"
          >
            <rect x="180" y="20" width="60" height="120" fill="black" />
            <rect x="240" y="20" width="60" height="120" fill="black" fillOpacity="0.5" />
          </mask>
        </defs>
        <rect x="20" y="20" width="120" height="120" fill="#176bba" mask="url(#luminance)" />
        <rect x="180" y="20" width="120" height="120" fill="#16a085" mask="url(#alpha)" />
      </svg>
    </View>
  );
}
