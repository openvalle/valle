export const composition = { width: 360, height: 210, fps: 1, duration: 1 };

// An <svg>'s viewBox in its box: fit and centered, the default; at its start, sliced; stretched;
// at its end. A width alone takes the height from the viewBox's ratio, a height alone in a
// flex row the width. Without a viewBox user units are px. The content box takes the drawing,
// inside padding and a border, and clips it with rounded corners; overflow: visible lets it out.
export default function Scene() {
  const drawing = (
    <>
      <rect x={-10} y={-10} width={60} height={40} fill="#ffcc80" />
      <rect width={40} height={20} fill="#3949ab" />
      <circle cx={20} cy={10} r={8} fill="#e53935" />
    </>
  );
  const box = { background: "#cfd8dc" };
  return (
    <View
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "flex-start",
        gap: 10,
        padding: 10,
        background: "#eceff1",
      }}
    >
      <svg width={60} height={60} viewBox="0 0 40 20" style={box}>
        {drawing}
      </svg>
      <svg
        width={60}
        height={60}
        viewBox="0 0 40 20"
        preserveAspectRatio="xMinYMin slice"
        style={box}
      >
        {drawing}
      </svg>
      <svg width={60} height={60} viewBox="0 0 40 20" preserveAspectRatio="none" style={box}>
        {drawing}
      </svg>
      <svg
        width={80}
        height={30}
        viewBox="0 0 40 20"
        preserveAspectRatio="xMaxYMax meet"
        style={box}
      >
        {drawing}
      </svg>
      <svg width={90} viewBox="0 0 40 20" style={box}>
        {drawing}
      </svg>
      <svg height={30} viewBox="0 0 40 20" style={box}>
        {drawing}
      </svg>
      <svg width={50} height={30} style={box}>
        {drawing}
      </svg>
      <svg
        width={80}
        height={50}
        viewBox="0 0 40 20"
        style={{ ...box, padding: 6, border: "4px solid #5c6bc0", borderRadius: 16 }}
      >
        {drawing}
      </svg>
      <svg
        width={60}
        height={40}
        viewBox="0 0 40 20"
        style={{ ...box, overflow: "visible", marginLeft: 10 }}
      >
        {drawing}
      </svg>
    </View>
  );
}
