export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <marker
            id="stroke"
            markerWidth="4"
            markerHeight="4"
            refX="2"
            refY="2"
            orient="auto"
            markerUnits="strokeWidth"
          >
            <rect width="4" height="4" fill="#ffb74d" />
          </marker>
          <marker
            id="user"
            markerWidth="12"
            markerHeight="12"
            refX="6"
            refY="6"
            orient="auto"
            markerUnits="userSpaceOnUse"
          >
            <rect width="12" height="12" fill="#16a085" />
          </marker>
        </defs>
        <path
          d="M30 30H130"
          fill="none"
          stroke="#176bba"
          strokeWidth="2"
          markerEnd="url(#stroke)"
        />
        <path
          d="M30 80H130"
          fill="none"
          stroke="#176bba"
          strokeWidth="6"
          markerEnd="url(#stroke)"
        />
        <path d="M200 30H300" fill="none" stroke="#176bba" strokeWidth="2" markerEnd="url(#user)" />
        <path d="M200 80H300" fill="none" stroke="#176bba" strokeWidth="6" markerEnd="url(#user)" />
      </svg>
    </View>
  );
}
