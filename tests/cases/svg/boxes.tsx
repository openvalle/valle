export const composition = { width: 400, height: 300, fps: 1, duration: 1 };

// An <svg> as a box among others: background, padding, a border and rounded corners, with a shadow;
// opacity, a blur and a drop shadow, which follows the drawing; a turn, and a turn in perspective;
// a blend over a gradient; a clip path and a mask image; absolute inside a box that clips it, with
// a z-index; sized by percentages, by flex growth, by a grid track and by an aspect ratio.
export default function Scene() {
  const star = "12 2 15 9 22 9 17 14 19 22 12 17 5 22 7 14 2 9 9 9";
  const shape = <polygon points={star} fill="#1e88e5" stroke="#0d47a1" />;
  const icon = { width: 56, height: 56, viewBox: "0 0 24 24" };
  const row = { display: "flex", gap: 10, marginBottom: 10 };
  return (
    <View style={{ padding: 10 }}>
      <View style={row}>
        <svg
          {...icon}
          style={{
            padding: 6,
            background: "#fff3e0",
            border: "3px solid #fb8c00",
            borderRadius: 12,
            boxShadow: "2px 3px 6px #00000066",
          }}
        >
          {shape}
        </svg>
        <svg {...icon} style={{ opacity: 0.5 }}>
          {shape}
        </svg>
        <svg {...icon} style={{ filter: "blur(1px)" }}>
          {shape}
        </svg>
        <svg {...icon} style={{ filter: "drop-shadow(2px 2px 2px #e53935)" }}>
          {shape}
        </svg>
        <svg {...icon} style={{ transform: "rotate(20deg)" }}>
          {shape}
        </svg>
      </View>
      <View style={row}>
        <View style={{ perspective: 120 }}>
          <svg {...icon} style={{ transform: "rotateY(40deg)", background: "#e8f5e9" }}>
            {shape}
          </svg>
        </View>
        <View style={{ background: "linear-gradient(90deg, #fdd835, #e53935)", padding: 4 }}>
          <svg {...icon} style={{ mixBlendMode: "multiply", display: "block" }}>
            {shape}
          </svg>
        </View>
        <svg {...icon} style={{ clipPath: "circle(40%)", background: "#f3e5f5" }}>
          {shape}
        </svg>
        <svg
          {...icon}
          style={{
            background: "#fff",
            maskImage: "linear-gradient(90deg, #000, transparent)",
          }}
        >
          {shape}
        </svg>
        <View
          style={{
            position: "relative",
            width: 56,
            height: 56,
            overflow: "hidden",
            background: "#cfd8dc",
          }}
        >
          <svg {...icon} style={{ position: "absolute", left: 20, top: 16, zIndex: 1 }}>
            {shape}
          </svg>
          <View style={{ position: "absolute", width: 40, height: 40, background: "#e53935" }} />
        </View>
      </View>
      <View style={{ ...row, height: 60 }}>
        <View style={{ width: 80, height: 60, background: "#eceff1" }}>
          <svg viewBox="0 0 24 24" style={{ width: "50%", height: "100%", display: "block" }}>
            {shape}
          </svg>
        </View>
        <svg viewBox="0 0 24 24" style={{ flexGrow: 1, height: 60, background: "#e3f2fd" }}>
          {shape}
        </svg>
        <View
          style={{
            display: "grid",
            gridTemplateColumns: "40px 1fr",
            width: 120,
            background: "#eee",
          }}
        >
          <svg viewBox="0 0 24 24" style={{ width: "100%", height: "100%" }}>
            {shape}
          </svg>
          <svg viewBox="0 0 24 24" preserveAspectRatio="none" style={{ width: "100%", height: 60 }}>
            {shape}
          </svg>
        </View>
      </View>
      <svg viewBox="0 0 24 12" style={{ width: 96, aspectRatio: "2 / 1", background: "#fce4ec" }}>
        <rect width={24} height={12} fill="none" stroke="#d81b60" />
        <circle cx={12} cy={6} r={4} fill="#d81b60" />
      </svg>
    </View>
  );
}
