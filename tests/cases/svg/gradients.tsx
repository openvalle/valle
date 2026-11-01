export const composition = { width: 360, height: 300, fps: 1, duration: 1 };

// SVG gradients: linear across the bounding box, the default; in user units, down, to a white
// stop with no opacity, which mixes unpremultiplied; turned by gradientTransform; padded,
// repeated and reflected past their stops; radial in a circle and stretched over a wide box; from
// a focus with a radius; one taking its stops and units from another by href, and by xlinkHref;
// a stroke; a curved path's tight bounding box; offsets that go back, a single stop, and
// currentColor; a gradient of another <svg>; and fill-opacity.
export default function Scene() {
  const box = { width: 100, height: 60 };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg {...box}>
        <defs>
          <linearGradient id="a">
            <stop offset="0" stopColor="#1e88e5" />
            <stop offset="1" stopColor="#fb8c00" />
          </linearGradient>
          <linearGradient id="b" gradientUnits="userSpaceOnUse" x2={0} y2={60}>
            <stop offset="0.1" stopColor="#3949ab" />
            <stop offset="0.9" stopColor="#ffffff" stopOpacity={0} />
          </linearGradient>
        </defs>
        <rect x={4} y={4} width={44} height={52} fill="url(#a)" />
        <rect x={52} y={4} width={44} height={52} fill="url(#b)" />
      </svg>
      <svg {...box}>
        <linearGradient id="c" gradientTransform="rotate(45 0.5 0.5)">
          <stop offset="0.2" stopColor="#e53935" />
          <stop offset="0.5" stopColor="#fdd835" />
          <stop offset="0.8" stopColor="#43a047" />
        </linearGradient>
        <rect x={4} y={4} width={92} height={52} rx={8} fill="url(#c)" />
      </svg>
      <svg {...box}>
        {(["pad", "repeat", "reflect"] as const).map((spread, k) => (
          <g key={spread}>
            <linearGradient id={`s${k}`} x1={0.3} x2={0.5} spreadMethod={spread}>
              <stop offset="0" stopColor="#8e24aa" />
              <stop offset="1" stopColor="#4dd0e1" />
            </linearGradient>
            <rect x={4} y={4 + k * 18} width={92} height={16} fill={`url(#s${k})`} />
          </g>
        ))}
      </svg>
      <svg {...box}>
        <radialGradient id="r">
          <stop offset="0" stopColor="#fff176" />
          <stop offset="0.6" stopColor="#f57c00" />
          <stop offset="1" stopColor="#6a1b9a" />
        </radialGradient>
        <circle cx={28} cy={30} r={24} fill="url(#r)" />
        <rect x={56} y={10} width={40} height={40} fill="url(#r)" />
      </svg>
      <svg {...box}>
        <radialGradient id="f" fx={0.3} fy={0.3} fr={0.1} spreadMethod="reflect" r={0.4}>
          <stop offset="0" stopColor="#ffffff" />
          <stop offset="1" stopColor="#00897b" />
        </radialGradient>
        <rect x={4} y={4} width={92} height={52} fill="url(#f)" />
      </svg>
      <svg {...box}>
        <linearGradient id="base" gradientUnits="userSpaceOnUse" x1={0} x2={100}>
          <stop offset="0" stopColor="#d81b60" />
          <stop offset="1" stopColor="#5e35b1" />
        </linearGradient>
        <linearGradient id="h1" href="#base" x2={50} />
        <linearGradient id="h2" xlinkHref="#base" spreadMethod="reflect" x2={25} />
        <rect x={4} y={4} width={92} height={24} fill="url(#h1)" />
        <rect x={4} y={32} width={92} height={24} fill="url(#h2)" />
      </svg>
      <svg {...box}>
        <circle cx={30} cy={30} r={20} fill="none" stroke="url(#a)" strokeWidth={10} />
        <path
          d="M70 18 C 70 6, 92 6, 92 22 C 92 36, 70 48, 70 56 C 70 48, 48 36, 48 22 C 48 6, 70 6, 70 18 Z"
          fill="url(#v)"
        />
        <linearGradient id="v" x1={0} x2={0} y2={1}>
          <stop offset="0" stopColor="#ef5350" />
          <stop offset="1" stopColor="#283593" />
        </linearGradient>
      </svg>
      <svg {...box} style={{ color: "#00acc1" }}>
        <linearGradient id="o">
          <stop offset="0.6" stopColor="#3949ab" />
          <stop offset="0.3" stopColor="currentColor" />
          <stop offset="1" stopColor="#fdd835" />
        </linearGradient>
        <linearGradient id="one">
          <stop offset="0.5" stopColor="#43a047" />
        </linearGradient>
        <rect x={4} y={4} width={92} height={24} fill="url(#o)" />
        <rect x={4} y={32} width={44} height={24} fill="url(#one)" />
        <rect x={52} y={32} width={44} height={24} fill="url(#a)" fillOpacity={0.4} />
      </svg>
    </View>
  );
}
