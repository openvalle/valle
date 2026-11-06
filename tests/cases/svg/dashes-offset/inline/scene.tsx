export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <path d="M20 30h280" fill="none" stroke="#176bba" strokeWidth="8" strokeDasharray="20 10" />
        <path
          d="M20 75h280"
          fill="none"
          stroke="#16a085"
          strokeWidth="8"
          strokeDasharray="20 10"
          strokeDashoffset="10"
        />
        <path
          d="M20 120h280"
          pathLength="28"
          fill="none"
          stroke="#ad1457"
          strokeWidth="8"
          strokeDasharray="2 1"
          strokeDashoffset="-1"
        />
      </svg>
    </View>
  );
}
