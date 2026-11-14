export const composition = { width: 300, height: 180, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 300, height: 180, backgroundColor: "white" }}>
      <svg xmlns="http://www.w3.org/2000/svg" width="300" height="180" viewBox="0 0 300 180">
        <defs>
          <rect id="tile" x="0" y="0" width="22" height="12" fill="#138a72" />
        </defs>
        <rect
          x="20"
          y="20"
          width="50"
          height="20"
          fill="#db504a"
          style={{
            transform: `rotate(${90 + seconds * 45}deg)`,
            transformOrigin: "center",
            transformBox: "fill-box",
          }}
        />
        <rect
          x="100"
          y="20"
          width="20"
          height="30"
          fill="#4455bb"
          transform="translate(-50)"
          style={{ transform: "none" }}
        />
        <g transform="translate(0,80)" style={{ transform: "translate(10px,80px)" }}>
          <rect x="20" y="0" width="40" height="20" fill="#e6a23b" />
        </g>
        <rect
          x="100"
          y="100"
          width="20"
          height="30"
          fill="#8dcdd0"
          stroke="#263238"
          strokeWidth="4"
          style={{
            transform: "scale(1.5)",
            transformOrigin: "top left",
            transformBox: "stroke-box",
          }}
        />
        <g
          style={{
            transform: "rotate(30deg)",
            transformOrigin: "center",
            transformBox: "fill-box",
          }}
        >
          <rect x="180" y="20" width="20" height="30" fill="#7a4da6" />
          <rect x="210" y="20" width="20" height="30" fill="#c36b9c" />
        </g>
        <use
          href="#tile"
          x="200"
          y="110"
          style={{
            translate: "10px 0px",
            transform: "rotate(90deg)",
            transformOrigin: "211px 116px",
          }}
        />
      </svg>
    </View>
  );
}
