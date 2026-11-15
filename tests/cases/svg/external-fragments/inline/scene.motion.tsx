export const composition = { width: 320, height: 160, fps: 1, duration: 1 };
export const controls = {
  resources: { shapes: resource({ kind: "image", default: "./shapes.svg" }) },
};
export default function Scene() {
  return (
    <View style={{ width: 320, height: 160, backgroundColor: "white" }}>
      <svg width="320" height="160">
        <defs>
          <rect id="tile" width="200" height="150" fill="red" />
        </defs>
        <use href="resource://shapes#tile" x="10" y="10" fill="#2878b8" />
        <use href="resource://shapes#tile" x="100" y="10" fill="#d06030" />
        <rect x="200" y="10" width="100" height="60" fill="url(resource://shapes#ink)" />
        <g clipPath="url(resource://shapes#crop)">
          <rect x="10" y="100" width="180" height="50" fill="#309060" />
        </g>
        <rect x="210" y="100" width="80" height="40" fill="url(resource://shapes#dots)" />
      </svg>
    </View>
  );
}
