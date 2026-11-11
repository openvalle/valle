// Aurora: a slow field of light from one WGSL shader, and a serif title that comes into focus
// letter by letter. Designed at 1920 x 1080, 30 fps, 5 s.

export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

// The fonts are Google Fonts' files through jsDelivr, at a pinned commit, under the SIL Open Font License 1.1.
export const controls = {
  resources: {
    Instrument: resource({
      kind: "font",
      default: [
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/instrumentserif/InstrumentSerif-Regular.ttf",
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/instrumentserif/InstrumentSerif-Italic.ttf",
      ],
    }),
    Geist: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geist/Geist%5Bwght%5D.ttf",
    }),
    GeistMono: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    }),
  },
};

const WHITE = "#F6F2EC";
// The small mono captions at the corners.
const LABEL = { fontFamily: "GeistMono", fontSize: 20, letterSpacing: 4 };
const TITLE_SIZE = 300;
const RING = { x: 1560, y: 372, r: 178 };

// Domain-warped noise: two warps feed a third field, and each paints one colour of the light.
const AURORA = `
struct Params { time: f32, gain: f32 }
@group(2) @binding(0) var<uniform> params: Params;

fn hash(p: vec2f) -> f32 {
  var q = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  q += dot(q, q.yzx + 33.33);
  return fract((q.x + q.y) * q.z);
}

fn noise(p: vec2f) -> f32 {
  let i = floor(p);
  let f = fract(p);
  let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
  return mix(
    mix(hash(i), hash(i + vec2f(1.0, 0.0)), u.x),
    mix(hash(i + vec2f(0.0, 1.0)), hash(i + vec2f(1.0, 1.0)), u.x),
    u.y,
  );
}

fn fbm(start: vec2f) -> f32 {
  var p = start;
  var sum = 0.0;
  var amp = 0.5;
  for (var i = 0; i < 5; i++) {
    sum += amp * noise(p);
    p = vec2f(p.x * 1.6 - p.y * 1.2, p.x * 1.2 + p.y * 1.6) + vec2f(3.1, 1.7);
    amp *= 0.5;
  }
  return sum;
}

fn shade(input: ShaderInput) -> vec4f {
  let aspect = input.size.x / input.size.y;
  let p = vec2f(input.uv.x * aspect, input.uv.y) * 0.95;
  let t = params.time * 0.11;
  let q = vec2f(fbm(p + vec2f(0.0, t)), fbm(p + vec2f(5.2, 1.3 - t)));
  let r = vec2f(
    fbm(p + 2.6 * q + vec2f(1.7, 9.2) + 1.3 * t),
    fbm(p + 2.6 * q + vec2f(8.3, 2.8) - 1.1 * t),
  );
  let f = fbm(p + 3.0 * r);

  var color = mix(vec3f(0.016, 0.014, 0.05), vec3f(0.10, 0.08, 0.46), smoothstep(0.2, 0.75, f));
  color = mix(color, vec3f(0.56, 0.13, 0.66), smoothstep(0.42, 0.78, q.x) * 0.85);
  color = mix(color, vec3f(1.0, 0.38, 0.17), smoothstep(0.5, 0.82, r.x) * 0.9);
  color += vec3f(0.04, 0.5, 0.56) * smoothstep(0.52, 0.8, r.y) * 0.4;
  color += vec3f(1.0, 0.82, 0.6) * smoothstep(0.7, 0.95, r.x * f * 2.0) * 0.35;

  // Darker toward the corners, so the light sits in the middle of the frame.
  let d = (input.uv - vec2f(0.56, 0.56)) * vec2f(1.0, 1.25);
  let vignette = 1.0 - smoothstep(0.25, 0.95, length(d));
  return vec4f(color * (0.3 + 0.7 * vignette) * params.gain, 1.0);
}`;

// 60 ticks around the ring, every fifth one longer.
const TICKS = Array.from({ length: 60 }, (_, i) => {
  const a = (i / 60) * Math.PI * 2;
  const inner = RING.r - (i % 5 === 0 ? 22 : 12);
  return {
    key: `tick-${i}`,
    x1: (inner * Math.cos(a)).toFixed(2),
    y1: (inner * Math.sin(a)).toFixed(2),
    x2: ((RING.r - 4) * Math.cos(a)).toFixed(2),
    y2: ((RING.r - 4) * Math.sin(a)).toFixed(2),
    major: i % 5 === 0,
  };
});

// One line of the title: every letter rises into place and comes into focus after the one before.
function Line({ text, start, italic }) {
  const { seconds } = useCurrentTime();
  return (
    <Text
      split="char"
      style={{
        fontFamily: "Instrument",
        fontStyle: italic ? "italic" : "normal",
        fontSize: TITLE_SIZE,
        lineHeight: 0.9,
        letterSpacing: -6,
        color: WHITE,
        whiteSpace: "pre",
      }}
      perUnit={(unit) => {
        const t = seconds - start - unit.index * 0.045;
        const arrive = interpolate(t, [0, 1.1], [0, 1], { easing: "easeOutExpo" });
        const sharp = interpolate(t, [0, 0.8], [0, 1], { easing: "easeOutCubic" });
        return {
          opacity: interpolate(t, [0, 0.5], [0, 1], { easing: "easeOutCubic" }),
          translate: `0px ${(1 - arrive) * 90}px`,
          filter: sharp < 1 ? `blur(${18 * (1 - sharp)}px)` : "none",
        };
      }}
    >
      {text}
    </Text>
  );
}

function Dial() {
  const { seconds } = useCurrentTime();
  const draw = interpolate(seconds, [1.1, 2.5], [0, 1], { easing: "easeInOutCubic" });
  const ticks = interpolate(seconds, [1.5, 2.5], [0, 1], { easing: "easeOutCubic" });
  const hand = -90 + 360 * (seconds / 5);
  return (
    <svg
      className="absolute"
      width={1}
      height={1}
      style={{ left: RING.x, top: RING.y, overflow: "visible" }}
    >
      <circle
        r={RING.r}
        fill="none"
        stroke="#F6F2EC8C"
        strokeWidth={1.5}
        pathLength={1}
        strokeDasharray="1 1"
        strokeDashoffset={1 - draw}
        transform="rotate(-90)"
      />
      <g opacity={ticks} transform={`rotate(${seconds * 6})`}>
        {TICKS.map((tick) => (
          <line
            key={tick.key}
            x1={tick.x1}
            y1={tick.y1}
            x2={tick.x2}
            y2={tick.y2}
            stroke={tick.major ? "#F6F2ECCC" : "#F6F2EC59"}
            strokeWidth={1.5}
          />
        ))}
      </g>
      <g opacity={ticks} transform={`rotate(${hand})`}>
        <line x1={0} y1={0} x2={RING.r - 34} y2={0} stroke="#F6F2ECB3" strokeWidth={1.5} />
        <circle cx={RING.r} cy={0} r={7} fill={WHITE} />
      </g>
      <circle r={4} fill={WHITE} opacity={ticks} />
    </svg>
  );
}

export default function Aurora() {
  const { seconds } = useCurrentTime();
  const gain = interpolate(seconds, [0, 1.4], [0, 1], { easing: "easeOutCubic" });
  const meta = interpolate(seconds, [0.5, 1.3], [0, 1], { easing: "easeOutCubic" });
  const rule = interpolate(seconds, [1.9, 2.9], [0, 1], { easing: "easeOutExpo" });
  const caption = interpolate(seconds, [2.2, 3.0], [0, 1], { easing: "easeOutCubic" });
  const push = 1 + 0.035 * (seconds / 5);
  return (
    <View className="relative h-full w-full" style={{ backgroundColor: "#05050A" }}>
      <ShaderLayer
        key="light"
        code={AURORA}
        uniforms={{ time: seconds, gain }}
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          width: 1920,
          height: 1080,
          filter: `film-grain(${Math.floor(seconds * 24)} 0.12 1.5px)`,
        }}
      />
      {/* A shade under the type, so the letters keep their contrast wherever the light goes. */}
      <View
        key="shade"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "linear-gradient(200deg, #05050A00 35%, #05050A8C 100%), linear-gradient(180deg, #05050A66 0%, #05050A00 22%)",
        }}
      />

      <View
        key="meta"
        className="absolute flex justify-between"
        style={{ left: 96, right: 96, top: 72, opacity: meta }}
      >
        <Text style={{ ...LABEL, color: "#F6F2ECB3" }}>N° 01 — AURORA</Text>
        <Text
          style={{
            fontFamily: "GeistMono",
            fontSize: 20,
            letterSpacing: 4,
            color: "#F6F2ECB3",
            fontVariantNumeric: "tabular-nums",
          }}
        >
          T + {formatNumber(seconds, { decimals: 2 })} S
        </Text>
      </View>

      <Dial key="dial" />

      <View
        key="title"
        className="absolute flex flex-col"
        style={{ left: 96, top: 356, scale: push, transformOrigin: "0% 100%" }}
      >
        <Line key="first" text="Light, in" start={0.35} />
        <Line key="second" text="motion." start={0.7} italic />
      </View>

      <View
        key="rule"
        className="absolute"
        style={{
          left: 100,
          top: 944,
          width: 1720,
          height: 1.5,
          backgroundColor: "#F6F2EC73",
          scale: `${rule} 1`,
          transformOrigin: "0% 50%",
        }}
      />
      <View
        key="caption"
        className="absolute flex justify-between"
        style={{
          left: 100,
          right: 100,
          top: 968,
          opacity: caption,
          translate: `0px ${(1 - caption) * 14}px`,
        }}
      >
        <Text style={{ fontFamily: "Geist", fontSize: 26, fontWeight: 400, color: "#F6F2ECD9" }}>
          Every frame is a pure function of time.
        </Text>
        <Text style={{ ...LABEL, color: "#F6F2EC99" }}>WGSL · 5 OCTAVES · 2 WARPS</Text>
      </View>
    </View>
  );
}
