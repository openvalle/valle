export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <rect x="20" y="20" width="70" height="50" rx="10" fill="#176bba" />
        <circle cx="140" cy="45" r="25" fill="#16a085" />
        <ellipse cx="220" cy="45" rx="40" ry="20" fill="#ffb74d" />
        <polygon points="20,145 60,95 100,145" fill="#ad1457" />
        <polyline
          points="130,140 155,100 180,140 205,100"
          fill="none"
          stroke="#176bba"
          strokeWidth="6"
        />
        <line x1="240" y1="100" x2="300" y2="145" stroke="#16a085" strokeWidth="6" />
      </svg>
    </View>
  );
}
