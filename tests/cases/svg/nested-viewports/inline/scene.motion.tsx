export const composition = { width: 420, height: 260, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 420, height: 260, background: "white" }}>
      <svg width="420" height="260" fontFamily="sans-serif">
        <rect x="10" y="10" width="120" height="90" fill="#e2e8f0" />
        <svg x="10" y="10" width={120 + seconds * 12} height="90" viewBox="0 0 60 30">
          <rect width="100%" height="100%" fill="#2563eb" />
          <circle cx="30" cy="15" r="20" fill="#fbbf24" />
        </svg>
        <svg
          x="150"
          y="10"
          width="110"
          height="90"
          viewBox="-10 -10 40 40"
          preserveAspectRatio="none"
        >
          <rect x="-10" y="-10" width="40" height="40" fill="#dbeafe" />
          <circle cx="10" cy="10" r="15" fill="#0891b2" />
          <path d="M-10 -10L30 30" stroke="#172554" strokeWidth="2" />
        </svg>
        <rect x="280" y="10" width="120" height="90" fill="#e2e8f0" />
        <svg x="280" y="10" width="120" height="90">
          <svg
            x="25%"
            y="25%"
            width="50%"
            height="50%"
            viewBox="0 0 10 10"
            preserveAspectRatio="none"
          >
            <rect width="100%" height="100%" fill="#c026d3" />
            <circle cx="5" cy="5" r="4" fill="#fae8ff" />
          </svg>
        </svg>
        <rect x="10" y="140" width="120" height="70" fill="#e2e8f0" />
        <svg
          x="10"
          y="140"
          width="120"
          height="70"
          viewBox="0 0 60 35"
          style={{ overflow: "visible" }}
        >
          <circle cx="30" cy="17.5" r="28" fill="#16a34a" />
        </svg>
        <svg
          x="160"
          y="140"
          width="10"
          height="70"
          style={{ width: "calc(25% + 10px)" }}
          viewBox="0 0 50 35"
          transform="rotate(5 160 140)"
        >
          <rect width="50" height="35" fill="#f97316" />
          <path d="M0 0L50 35M0 35L50 0" stroke="white" strokeWidth="3" />
        </svg>
        <svg x="305" y="135" width="80" height="80">
          <rect width="100%" height="100%" fill="#f1f5f9" />
          <text x="50%" y="50%" textAnchor="middle" dominantBaseline="middle" fontSize="22">
            SVG
          </text>
        </svg>
        <svg x="0" y="0" width="0" height="260">
          <rect width="1000" height="1000" fill="red" />
        </svg>
        <svg width="420" height="260" viewBox="0 0 0 10">
          <rect width="1000" height="1000" fill="red" />
        </svg>
      </svg>
    </View>
  );
}
