export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <g transform="translate(20 20)" fill="#176bba">
          <rect width="60" height="40" />
          <g transform="translate(20 60) scale(2 1)">
            <rect width="40" height="40" fill="#16a085" />
          </g>
        </g>
        <g transform="translate(200 30) rotate(90)">
          <rect width="80" height="40" fill="#ffb74d" />
        </g>
        <g opacity="0.5">
          <rect x="240" y="20" width="60" height="80" fill="red" />
          <rect x="260" y="60" width="60" height="80" fill="blue" />
        </g>
      </svg>
    </View>
  );
}
