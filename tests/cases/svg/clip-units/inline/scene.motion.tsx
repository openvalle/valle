export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <clipPath id="user" clipPathUnits="userSpaceOnUse">
            <rect x="30" y="30" width="80" height="100" />
          </clipPath>
          <clipPath id="bounds" clipPathUnits="objectBoundingBox">
            <rect x="0.25" y="0.25" width="0.5" height="0.5" />
          </clipPath>
        </defs>
        <rect x="20" y="20" width="120" height="120" fill="#176bba" clipPath="url(#user)" />
        <rect x="180" y="20" width="120" height="120" fill="#16a085" clipPath="url(#bounds)" />
      </svg>
    </View>
  );
}
