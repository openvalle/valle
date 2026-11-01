export const composition = { width: 360, height: 290, fps: 1, duration: 1 };

// SVG clip paths and masks: a circle clipping a group in user units; a star in fractions of the
// bounding box, even-odd; a turned clip path of two shapes, one moved; an ellipse over a group's
// bounding box; a clip rule its shapes inherit, with hidden shapes left out, by style; masks by
// luminance and by alpha; a mask's content in fractions of the box; a region in user units, and
// the default one cutting a stroke; a gradient mask with a clip path and opacity on one group;
// and a mask whose content is masked, on a transformed shape.
export default function Scene() {
  const box = { width: 100, height: 60 };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 8, padding: 8, background: "#eceff1" }}>
      <svg {...box}>
        <clipPath id="c1">
          <circle cx={50} cy={30} r={24} />
        </clipPath>
        <g clipPath="url(#c1)">
          <rect width={50} height={60} fill="#1e88e5" />
          <rect x={50} width={50} height={60} fill="#fb8c00" />
        </g>
      </svg>
      <svg {...box}>
        <clipPath id="c2" clipPathUnits="objectBoundingBox">
          <polygon points="0.5,0 0.81,0.95 0.02,0.36 0.98,0.36 0.19,0.95" clipRule="evenodd" />
        </clipPath>
        <rect x={20} y={4} width={60} height={52} fill="#43a047" clipPath="url(#c2)" />
      </svg>
      <svg {...box}>
        <clipPath id="c3" transform="rotate(15 50 30)">
          <rect x={10} y={10} width={40} height={40} transform="translate(-4 0)" />
          <circle cx={66} cy={30} r={20} />
        </clipPath>
        <rect width={100} height={60} fill="#8e24aa" clipPath="url(#c3)" />
      </svg>
      <svg {...box}>
        <clipPath id="c4" clipPathUnits="objectBoundingBox">
          <ellipse cx={0.5} cy={0.5} rx={0.5} ry={0.5} />
        </clipPath>
        <g clipPath="url(#c4)">
          <rect x={10} y={10} width={30} height={30} fill="#e53935" />
          <rect width={30} height={20} fill="#fdd835" transform="translate(60 34)" />
        </g>
      </svg>
      <svg {...box}>
        <clipPath id="c5" clipRule="evenodd">
          <path d="M10 6 H90 V54 H10 Z M30 18 H70 V42 H30 Z" />
          <circle cx={50} cy={30} r={8} visibility="hidden" />
          <rect width={100} height={60} style={{ display: "none" }} />
        </clipPath>
        <rect width={100} height={60} fill="#00897b" style={{ clipPath: "url(#c5)" }} />
      </svg>
      <svg {...box}>
        <mask id="m1">
          <rect width={100} height={60} fill="white" />
          <circle cx={30} cy={30} r={14} fill="black" />
          <rect x={56} y={10} width={36} height={40} fill="#808080" />
        </mask>
        <rect width={100} height={60} fill="#3949ab" mask="url(#m1)" />
      </svg>
      <svg {...box}>
        <mask id="m2" maskType="alpha">
          <rect x={4} y={4} width={44} height={52} fill="black" />
          <rect x={52} y={4} width={44} height={52} fill="white" fillOpacity={0.3} />
        </mask>
        <rect width={100} height={60} fill="#e53935" mask="url(#m2)" />
      </svg>
      <svg {...box}>
        <mask id="m3" maskContentUnits="objectBoundingBox">
          <circle cx={0.5} cy={0.5} r={0.4} fill="white" />
        </mask>
        <rect x={20} y={10} width={60} height={40} fill="#f57c00" mask="url(#m3)" />
      </svg>
      <svg {...box}>
        <mask id="m4" maskUnits="userSpaceOnUse" x={20} y={10} width={60} height={40}>
          <rect width={100} height={60} fill="white" />
        </mask>
        <circle cx={50} cy={30} r={28} fill="#43a047" mask="url(#m4)" />
      </svg>
      <svg {...box}>
        <mask id="m5">
          <rect width={100} height={60} fill="white" />
        </mask>
        <rect
          x={30}
          y={15}
          width={40}
          height={30}
          fill="#fdd835"
          stroke="#6a1b9a"
          strokeWidth={12}
          mask="url(#m5)"
        />
      </svg>
      <svg {...box}>
        <linearGradient id="g6">
          <stop offset="0" stopColor="white" />
          <stop offset="1" stopColor="black" />
        </linearGradient>
        <mask id="m6">
          <rect width="100%" height="100%" fill="url(#g6)" />
        </mask>
        <clipPath id="c6">
          <rect x={4} y={4} width={92} height={52} rx={14} />
        </clipPath>
        <g mask="url(#m6)" clipPath="url(#c6)" opacity={0.8}>
          <rect width={100} height={60} fill="#1e88e5" />
          <circle cx={70} cy={30} r={20} fill="#e53935" />
        </g>
      </svg>
      <svg {...box}>
        <mask id="m7">
          <circle cx={20} cy={20} r={16} fill="white" />
        </mask>
        <mask id="m8">
          <rect width={100} height={60} fill="white" mask="url(#m7)" />
        </mask>
        <rect
          width={40}
          height={40}
          fill="#00897b"
          transform="translate(30 10) rotate(10)"
          mask="url(#m8)"
        />
      </svg>
    </View>
  );
}
