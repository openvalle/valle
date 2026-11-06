export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <marker
            id="arrow"
            viewBox="0 0 10 10"
            refX="5"
            refY="5"
            markerWidth="20"
            markerHeight="20"
            markerUnits="userSpaceOnUse"
            orient="auto-start-reverse"
          >
            <path d="M0 0L10 5L0 10z" fill="#176bba" />
          </marker>
        </defs>
        <path
          d="M40 40H280"
          fill="none"
          stroke="#176bba"
          strokeWidth="4"
          markerStart="url(#arrow)"
          markerEnd="url(#arrow)"
        />
        <path
          d="M40 140L160 80L280 140"
          fill="none"
          stroke="#16a085"
          strokeWidth="4"
          markerMid="url(#arrow)"
          markerEnd="url(#arrow)"
        />
      </svg>
    </View>
  );
}
