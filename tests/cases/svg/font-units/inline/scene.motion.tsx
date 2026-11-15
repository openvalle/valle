export const composition = { width: 320, height: 160, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 320, height: 160, backgroundColor: "white" }}>
      <svg width="320" height="160" fontSize="20">
        <defs>
          <rect id="tile" width="2em" height="1em" />
        </defs>
        <g fontSize="10">
          <rect x="1em" y="1em" width="6em" height="2em" fill="#2878b8" />
        </g>
        <g fontSize="20">
          <rect x="5em" y="1em" width="3em" height="2em" fill="#d06030" />
        </g>
        <use href="#tile" x="20" y="90" fontSize="30" fill="#2878b8" />
        <use href="#tile" x="110" y="90" fontSize="20" fill="#d06030" />
        <rect x="200" y="90" width="1.5em" height="1em" fontSize="32" fill="#309060" />
        <svg x="200" y="10" width="100" height="60" viewBox="0 0 50 30" fontSize="10">
          <rect width="3em" height="2em" fill="#309060" />
        </svg>
      </svg>
    </View>
  );
}
