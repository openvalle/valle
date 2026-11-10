export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <g id="tile">
            <rect width="40" height="40" />
            <rect x="10" y="10" width="20" height="20" fill="#ffb74d" />
          </g>
          <symbol id="badge" viewBox="0 0 40 40">
            <rect width="40" height="40" fill="#16a085" />
            <path d="M10 20h20M20 10v20" stroke="white" strokeWidth="4" />
          </symbol>
        </defs>
        <use href="#tile" x="20" y="30" fill="#176bba" />
        <use href="#tile" x="90" y="30" fill="#ad1457" />
        <use href="#badge" x="160" y="20" width="80" height="120" />
        <use href="#badge" x="260" y="20" width="40" height="120" />
      </svg>
    </View>
  );
}
