export const composition = { width: 360, height: 160, fps: 1, duration: 1 };

// SVG <use> and <symbol>: copies of a turned group moved by x and y, filling as each <use> says;
// a symbol's viewBox fit to the viewport each <use> gives it, in its color; symbols clipped to
// their viewport, and sliced; a <use> of a <use>, whose copies inherit from both, and of a symbol
// in another <svg>; and a <use> with a gradient, a transform, and clip paths in its moved units.
export default function Scene() {
  const box = { width: 100, height: 60 };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg {...box}>
        <defs>
          <g id="u1" transform="rotate(10 20 20)">
            <rect x={8} y={8} width={24} height={24} rx={4} />
            <circle cx={20} cy={20} r={6} fill="#fff" />
          </g>
        </defs>
        <use href="#u1" fill="#1e88e5" />
        <use href="#u1" x={30} fill="#e53935" />
        <use href="#u1" x={60} y={16} style={{ fill: "#43a047" }} opacity={0.6} />
      </svg>
      <svg {...box}>
        <symbol id="u2" viewBox="0 0 24 24">
          <path
            d="M12 2 L15 9 L22 9 L16.5 13.5 L18.5 21 L12 16.5 L5.5 21 L7.5 13.5 L2 9 L9 9 Z"
            fill="currentColor"
          />
        </symbol>
        <use href="#u2" x={4} y={4} width={24} height={24} color="#fb8c00" />
        <use href="#u2" x={30} y={4} width={52} height={52} color="#8e24aa" />
        <use href="#u2" x={84} y={36} width={12} height={20} color="#00897b" />
      </svg>
      <svg {...box}>
        <symbol id="u3" viewBox="0 0 10 10">
          <circle cx={5} cy={5} r={6.5} fill="#3949ab" />
        </symbol>
        <symbol id="u3b" viewBox="0 0 10 10" preserveAspectRatio="xMinYMid slice">
          <rect width={10} height={10} fill="#fdd835" stroke="#6a1b9a" strokeWidth={2} />
        </symbol>
        <use href="#u3" x={6} y={10} width={40} height={40} />
        <use href="#u3b" x={54} y={20} width={40} height={20} />
      </svg>
      <svg {...box}>
        <defs>
          <circle id="u4" cx={14} cy={30} r={10} fill="none" />
          <use id="u4b" href="#u4" stroke="#e53935" strokeWidth={4} />
        </defs>
        <use href="#u4b" />
        <use href="#u4b" x={26} strokeDasharray="4 2" />
        <use href="#u2" x={66} y={14} width={32} height={32} color="#43a047" />
      </svg>
      <svg {...box}>
        <defs>
          <linearGradient id="u5g">
            <stop offset="0" stopColor="#f57c00" />
            <stop offset="1" stopColor="#6a1b9a" />
          </linearGradient>
          <rect id="u5" width={36} height={36} fill="url(#u5g)" />
        </defs>
        <clipPath id="u5c">
          <circle cx={18} cy={18} r={18} />
        </clipPath>
        <use href="#u5" x={4} y={12} />
        <use href="#u5" x={10} transform="translate(36 6) rotate(15)" clipPath="url(#u5c)" />
      </svg>
    </View>
  );
}
