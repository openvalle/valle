export const composition = { width: 360, height: 200, fps: 1, duration: 1 };

// One sprite sheet, reused: symbols in a sheet of no size, drawn from another <svg> at several
// sizes and colors; a symbol of gradient paint, a clip path and a marker; copies that override
// what they inherit, and shapes in a symbol that keep their own; a <use> of a group inside a
// transformed group, with its opacity; and a grid of copies made in a loop.
export default function Scene() {
  const row = { display: "flex", alignItems: "center", gap: 8, marginBottom: 8 };
  const use = (href, size, style) => (
    <svg width={size} height={size} style={style}>
      <use href={href} width={size} height={size} />
    </svg>
  );
  return (
    <View style={{ padding: 8 }}>
      <svg width={0} height={0} style={{ position: "absolute" }}>
        <defs>
          <linearGradient id="sheen" x1={0} y1={0} x2={0} y2={1}>
            <stop offset={0} stopColor="#90caf9" />
            <stop offset={1} stopColor="#0d47a1" />
          </linearGradient>
          <clipPath id="round">
            <circle cx={12} cy={12} r={10} />
          </clipPath>
          <marker
            id="tip"
            viewBox="0 0 10 10"
            refX={8}
            refY={5}
            markerWidth={4}
            markerHeight={4}
            orient="auto"
          >
            <path d="M0 0 L10 5 L0 10 Z" fill="#e53935" />
          </marker>
          <g id="pair">
            <rect x={-12} y={-12} width={11} height={24} />
            <rect x={1} y={-12} width={11} height={24} fill="#fdd835" />
          </g>
        </defs>
        <symbol id="plus" viewBox="0 0 24 24">
          <path d="M12 4 V20 M4 12 H20" fill="none" stroke="currentColor" strokeWidth={4} />
        </symbol>
        <symbol id="drop" viewBox="0 0 24 24">
          <path d="M12 2 C12 2 4 11 4 16 A8 8 0 0 0 20 16 C20 11 12 2 12 2 Z" fill="url(#sheen)" />
        </symbol>
        <symbol id="split" viewBox="0 0 24 24">
          <g clipPath="url(#round)">
            <rect width={12} height={24} fill="#43a047" />
            <rect x={12} width={12} height={24} fill="currentColor" />
          </g>
        </symbol>
        <symbol id="arrow" viewBox="0 0 24 24">
          <path
            d="M4 20 Q6 6 18 6"
            fill="none"
            stroke="#263238"
            strokeWidth={2}
            markerEnd="url(#tip)"
          />
        </symbol>
      </svg>
      <View style={row}>
        {use("#plus", 16, { color: "#e53935" })}
        {use("#plus", 24, { color: "#43a047" })}
        {use("#plus", 40, { color: "#1e88e5" })}
        {use("#drop", 24)}
        {use("#drop", 48)}
        {use("#split", 40, { color: "#fb8c00" })}
        {use("#split", 40, { color: "#8e24aa" })}
        {use("#arrow", 48)}
      </View>
      <View style={row}>
        <svg width={120} height={60} viewBox="-30 -30 120 60">
          <use href="#pair" fill="#e53935" />
          <use href="#pair" x={48} fill="#1e88e5" opacity={0.6} />
        </svg>
        <svg width={120} height={60} viewBox="0 0 120 60">
          <g transform="translate(30 30) rotate(30)">
            <use href="#pair" fill="#00897b" transform="scale(0.8)" />
            <use href="#pair" x={40} fill="#8e24aa" />
          </g>
        </svg>
        <svg width={88} height={60} viewBox="0 0 88 60" color="#3949ab">
          {[0, 1, 2, 3].map((x) =>
            [0, 1].map((y) => (
              <use
                key={`${x}-${y}`}
                href="#plus"
                x={x * 22}
                y={y * 30}
                width={20}
                height={20}
                opacity={1 - (x + y) * 0.15}
              />
            )),
          )}
        </svg>
      </View>
    </View>
  );
}
