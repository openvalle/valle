// Memphis loop: tiles turn in a diagonal wave, squiggles slip along the gaps, sprinkles spin, the
// dot field ripples and big shapes drift at the corners; every motion repeats a whole number of
// times in six seconds, so the loop has no seam. Shows loop: true, .map instances in inline SVG,
// mod() for wrapping, pingPong, Math.sin/cos, noise2d, seededRandom, wiggle2D, field(), zigzag(),
// twist(), puckerBloat() and roundCorners().
// Render: valle motion render examples/06-memphis-loop.motion.tsx -o 06-memphis-loop.mp4

export const composition = { width: 1080, height: 1080, fps: 60, duration: 6, loop: true };

// Easings.
const IN_OUT_BACK = "cubic-bezier(0.68, -0.6, 0.32, 1.6)";

const BLUSH = "#FFF0F5";
const PINK = "#FF6F91";
const YELLOW = "#FFC75F";
const MINT = "#00C9A7";
const PURPLE = "#845EC2";
const BLUE = "#2C73D2";
const INK = "#2E2350";
const COLORS = [PINK, YELLOW, MINT, PURPLE, BLUE];

// Tile shapes, each drawn in a 170 x 170 box around its centre. Each shape has its own colour,
// chosen so no tile matches the corner shape it overlaps.
const SHAPES = [
  { d: "M 17 85 A 68 68 0 1 1 153 85 A 68 68 0 1 1 17 85 Z", fill: PURPLE },
  { d: roundCorners("M 27 27 L 143 27 L 143 143 L 27 143 Z", 14), fill: PINK },
  { d: roundCorners("M 85 7 L 152 124 L 18 124 Z", 7), fill: BLUE },
  { d: "M 12 112 A 73 73 0 0 1 158 112 Z", fill: MINT },
  {
    d: roundCorners(
      "M 63 15 L 107 15 L 107 63 L 155 63 L 155 107 L 107 107 L 107 155 L 63 155 L 63 107 L 15 107 L 15 63 L 63 63 Z",
      6,
    ),
    fill: YELLOW,
  },
];

// A 4 x 4 grid: the shape shifts along rows and columns so no neighbours match, and the delay
// grows along the diagonal.
const TILES = Array.from({ length: 16 }, (_, i) => {
  const col = i % 4;
  const row = Math.floor(i / 4);
  return {
    id: `tile-${i}`,
    left: 50 + col * 270,
    top: 50 + row * 270,
    shape: SHAPES[(col + 2 * row) % 5],
    delay: (col + row) * 0.1,
  };
});

const DOTS = Array.from({ length: 27 * 27 }, (_, i) => ({
  id: `dot-${i}`,
  x: 20 + (i % 27) * 40,
  y: 20 + Math.floor(i / 27) * 40,
}));

// Sprinkles sit in the vertical gaps between tiles, nudged and turned by seeded random values.
const NUDGE = seededRandom(11, 24, [-18, 18]);
const START_ANGLE = seededRandom(12, 12, [0, 180]);
const SPRINKLE = roundCorners("M -26 -8 L 26 -8 L 26 8 L -26 8 Z", 8);
const SPRINKLE_AT = Array.from({ length: 12 }, (_, i) => ({
  x: 270 * (1 + (i % 3)) + NUDGE[2 * i],
  y: 135 + 270 * Math.floor(i / 3) + NUDGE[2 * i + 1],
}));
const SPRINKLE_FILL = Array.from({ length: 12 }, (_, i) => COLORS[(i * 2 + 1) % 5]);
const END_ANGLE = START_ANGLE.map((angle, i) => angle + (i % 2 === 0 ? 360 : -360));

// Squiggles travel the horizontal gaps, one per gap, alternating direction.
const SQUIGGLE = roundCorners(zigzag("M 0 18 L 240 18", { size: 14, ridges: 9 }), 10);
const SQUIGGLES = [0, 1, 2];

const PLUS =
  "M -55 -170 L 55 -170 L 55 -55 L 170 -55 L 170 55 L 55 55 L 55 170 L -55 170 L -55 55 L -170 55 L -170 -55 L -55 -55 Z";
// A twelve-point star; puckerBloat() swings it between a round sun and a sharp sunburst.
const SUNBURST = `M ${Array.from({ length: 24 }, (_, i) => {
  const radius = i % 2 === 0 ? 250 : 196;
  return `${radius * Math.cos((i * (2 * Math.PI)) / 24)} ${radius * Math.sin((i * (2 * Math.PI)) / 24)}`;
}).join(" L ")} Z`;

// A tile turns a quarter every 1.5 s, with a little anticipation and overshoot, and swells as it
// turns; it repeats four times a loop. Its shadow turns with it but stays offset down and right.
function Tile({ tile }) {
  const { seconds } = useCurrentTime();
  const beat = (seconds - tile.delay) / 1.5;
  const turn = interpolate(beat - Math.floor(beat), [0, 0.55], [0, 1], { easing: IN_OUT_BACK });
  const swell = 1 + 0.14 * Math.sin(interpolate(beat - Math.floor(beat), [0, 0.55], [0, Math.PI]));
  const box = {
    left: tile.left,
    top: tile.top,
    width: 170,
    height: 170,
    rotate: `${90 * (Math.floor(beat) + turn)}deg`,
    scale: swell,
  };
  return (
    <>
      <svg
        key="shadow"
        className="absolute"
        width={170}
        height={170}
        style={{ ...box, translate: "10px 10px" }}
      >
        <path d={tile.shape.d} fill={INK} />
      </svg>
      <svg key="face" className="absolute" width={170} height={170} style={box}>
        <path d={tile.shape.d} fill={tile.shape.fill} />
      </svg>
    </>
  );
}

// A squiggle crosses its gap once a loop, from one side to the other; mod() spaces the three a
// third of a loop apart and wraps each back to its start off screen.
function Squiggle({ index }) {
  const { progress } = useCurrentTime();
  const along = mod(progress + index / 3, 1);
  const x = index % 2 === 0 ? -290 + 1420 * along : 1130 - 1420 * along;
  return (
    <svg
      className="absolute"
      width={240}
      height={36}
      style={{ left: 0, top: 252 + 270 * index, overflow: "visible", translate: `${x}px 0px` }}
    >
      <path
        d={SQUIGGLE}
        fill="none"
        stroke={INK}
        strokeWidth={13}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

export default function MemphisLoop() {
  const { seconds, progress } = useCurrentTime();
  // One turn of the loop as an angle: anything driven by sin or cos of it repeats seamlessly.
  const cycle = (seconds * (2 * Math.PI)) / 6;

  // The sun and the ring wander by sampling noise around a circle, which ends where it starts.
  const wander = (seed) => 70 * (noise2d(seed, 1.6 * Math.cos(cycle), 1.6 * Math.sin(cycle)) - 0.5);
  // wiggle2D() is noise in time on both axes; reading it at pingPong(seconds, 3) plays it forth
  // and back.
  const sway = wiggle2D(pingPong(seconds, 3), { seed: 9, frequency: 0.4, amplitude: [26, 18] });
  // Each sprinkle spins a whole turn a loop, one way or the other.
  const turns = field({ from: START_ANGLE, to: END_ANGLE, progress });

  return (
    <View className="relative h-full w-full" style={{ backgroundColor: BLUSH }}>
      <svg className="absolute inset-0" width={1080} height={1080}>
        {DOTS.map((dot) => (
          <circle
            key={dot.id}
            cx={dot.x}
            cy={dot.y}
            fill="#2E235040"
            r={3.4 + 2.2 * Math.sin(cycle * 2 - (dot.x + dot.y) * 0.006)}
          />
        ))}
        <path
          key="sun"
          d={puckerBloat(SUNBURST, 0.2 + 0.6 * Math.sin(cycle * 2))}
          fill={YELLOW}
          transform={`translate(${105 + wander(3)} ${115 + wander(4)}) rotate(${15 * Math.sin(cycle)})`}
        />
        <path
          key="pinwheel"
          d={twist(PLUS, 0.9 * Math.sin(cycle))}
          fill={PINK}
          transform="translate(965 120)"
        />
        <circle
          key="ring"
          cx={1010 + wander(5)}
          cy={1000 + wander(6)}
          r={250}
          fill="none"
          stroke={PURPLE}
          strokeWidth={64}
        />
        <path
          key="half-moon"
          fill={MINT}
          d="M -230 0 A 230 230 0 0 1 230 0 Z"
          transform={`translate(${95 + sway.x} ${1010 + sway.y}) rotate(${35 + 14 * Math.sin(cycle)})`}
        />
        {turns.map((angle, i) => (
          <path
            key={`sprinkle-${i}`}
            d={SPRINKLE}
            fill={SPRINKLE_FILL[i]}
            transform={`translate(${SPRINKLE_AT[i].x} ${SPRINKLE_AT[i].y}) rotate(${angle})`}
          />
        ))}
      </svg>

      {TILES.map((tile) => (
        <Tile key={tile.id} tile={tile} />
      ))}

      {SQUIGGLES.map((index) => (
        <Squiggle key={`squiggle-${index}`} index={index} />
      ))}
    </View>
  );
}
