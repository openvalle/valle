export const composition = { width: 320, height: 220, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 320, height: 220, backgroundColor: "#dae9dd" }}>
      <svg xmlns="http://www.w3.org/2000/svg" width="320" height="220" viewBox="0 0 320 220">
        <defs>
          <clipPath id="clip">
            <rect x="205" y="115" width="80" height="75" rx="12" />
          </clipPath>
          <mask id="mask" maskUnits="userSpaceOnUse" x="205" y="115" width="80" height="75">
            <rect x="205" y="115" width="80" height="75" fill="white" />
            <circle cx="250" cy="150" r="12" fill="black" />
          </mask>
        </defs>
        <rect x="10" y="10" width="80" height="70" fill="#51b9db" />
        <g style={{ mixBlendMode: "multiply" }}>
          <circle cx="50" cy="45" r="25" fill="#f25f5c" />
        </g>
        <rect x="110" y="10" width="80" height="70" fill="#51b9db" />
        <g style={{ isolation: "isolate" }}>
          <circle cx="150" cy="45" r="25" fill="#f25f5c" style={{ mixBlendMode: "multiply" }} />
        </g>
        <circle cx="260" cy="45" r="25" fill="#f25f5c" style={{ mixBlendMode: "multiply" }} />
        <g style={{ opacity: "0.6", filter: `blur(${2 + seconds * 2}px)` }}>
          <rect x="15" y="120" width="35" height="40" fill="#3874cb" />
          <rect
            x="35"
            y="135"
            width="35"
            height="40"
            fill="#e1bc29"
            style={{ mixBlendMode: "multiply" }}
          />
        </g>
        <path
          d="M115 130L160 115L170 165Z"
          fill="#a04eb5"
          style={{
            filter: "drop-shadow(4px 6px 2px #28415a) saturate(0.5)",
            transform: "translate(0px,5px)",
          }}
        />
        <g
          clipPath="url(#clip)"
          mask="url(#mask)"
          style={{ opacity: "0.7", filter: "blur(3px)", mixBlendMode: "multiply" }}
        >
          <rect x="195" y="105" width="100" height="90" fill="#58a47e" />
          <rect x="240" y="130" width="50" height="55" fill="#de7575" />
        </g>
      </svg>
    </View>
  );
}
