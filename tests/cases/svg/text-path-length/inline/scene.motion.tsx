export const composition = { width: 320, height: 180, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 320, height: 180, backgroundColor: "white" }}>
      <svg width="320" height="180" fontFamily="sans-serif" fontSize="24">
        <defs>
          <path id="a" d="M10 40H210" pathLength="100" />
          <path id="b" d="M10 80H210" />
          <path id="c" d="M10 120H210" pathLength="50" />
        </defs>
        <path d="M50 10V145M10 40H250M10 80H250M10 120H250" fill="none" stroke="#ddd" />
        <text fill="#2878b8">
          <textPath href="#a" startOffset="20">
            HI
          </textPath>
        </text>
        <text fill="#2878b8">
          <textPath href="#b" startOffset="40">
            HI
          </textPath>
        </text>
        <text fill="#309060">
          <textPath href="#c" startOffset="20%">
            HI
          </textPath>
        </text>
      </svg>
    </View>
  );
}
