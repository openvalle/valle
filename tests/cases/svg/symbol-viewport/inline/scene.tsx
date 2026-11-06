export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <symbol id="tile" viewBox="0 0 100 50" preserveAspectRatio="xMidYMid meet">
            <rect width="100" height="50" fill="#176bba" />
            <rect x="25" y="10" width="50" height="30" fill="#ffb74d" />
          </symbol>
          <clipPath id="clip">
            <rect x="180" y="40" width="110" height="80" />
          </clipPath>
        </defs>
        <use href="#tile" x="20" y="20" width="140" height="140" />
        <g clipPath="url(#clip)" opacity="0.5">
          <use href="#tile" x="180" y="20" width="140" height="140" />
        </g>
      </svg>
    </View>
  );
}
