# Motion scenes

A Motion scene is a `*.motion.tsx` file: React-style components with CSS, Tailwind classes and
inline SVG, drawn frame by frame. It is compiled, not run in a browser: every frame is a pure
function of the time and the scene's inputs, so the same scene draws the same pixels on every
machine. `valle motion check` compiles it and checks every frame; `valle motion render` draws
it.

Scene entries use the `.motion.tsx` suffix, including components used as Timeline clips or
caption presenters. Imported helpers keep ordinary names such as `Card.tsx` and `math.ts`;
they are modules, not entries. CLI and browser playback reject other entry suffixes.

## A first scene

```tsx scene title.motion.tsx
export const composition = { width: 1920, height: 1080, fps: 30, duration: 3 };

export default function Title() {
  const { seconds } = useCurrentTime();
  const reveal = interpolate(seconds, [0, 0.8], [0, 1], { easing: "easeOutCubic" });
  return (
    <View className="flex h-full w-full items-center justify-center bg-slate-900">
      <Text
        style={{
          fontSize: 120,
          fontWeight: 700,
          color: "white",
          opacity: reveal,
          transform: `translateY(${(1 - reveal) * 40}px)`,
        }}
      >
        Hello, Valle
      </Text>
    </View>
  );
}
```

```sh
valle motion check title.motion.tsx
valle motion render title.motion.tsx --frames 0,15,45,89 --storyboard -o sheet.png
valle motion render title.motion.tsx -o title.mp4
valle motion render title.motion.tsx --fps 60 --output-size 3840x2160 -o title-4k.mp4
```

Look at individual frames with `--frame N`, or at several frames together with `--storyboard`.

`render --fps 60` samples the same animation at 60 frames per second without rewriting the
TSX. `useComposition().fps`, motion blur and shutter samples use that output rate. Direct
reads of the exported `composition.fps` still give the authored value; use the hook when
you need the actual rate. Frame numbers in `--frame` and `--frames` refer to the output:
frame 60 is at one second when rendering at 60 fps. Without `--fps`, the authored rate applies.

Integer, decimal and fractional rates are accepted. `29.97` means exactly 2997/100;
use `30000/1001` for the NTSC rate. Duration and animation timing stay in seconds. As with
the authored rate, a video contains all frames starting before its duration, so its encoded
duration can extend by less than one output frame to complete the last frame.

## Storyboards

`--storyboard` defaults to 12 evenly spaced frames, including the first and last. Shorter
scenes use every frame. Choose how to sample and how many frames fit on each PNG separately:

```sh
valle motion render title.motion.tsx --storyboard -o sheet.png
valle motion render title.motion.tsx --storyboard --samples 60 --per-page 12 -o sheet.png
valle motion render title.motion.tsx --storyboard --step 30 --per-page 8 -o sheet.png
valle motion render title.motion.tsx --storyboard --frames 0..89 --per-page 12 -o sheet.png
```

- `--samples N`: evenly sample up to N frames; `1` selects the first frame.
- `--step N`: take frames 0, N, 2N, and so on. `--step 1` takes every frame.
- `--frames A,B,…` or `--frames A..B`: select exact frames, preserving order and duplicates.
  A range includes both ends; frame numbers start at zero. `--frame N` selects just one.
- `--per-page N`: at most N frames per image, default 12.

Sampling modes are mutually exclusive. `--samples`, `--step` and `--per-page` require
`--storyboard`, and their values must be positive integers. The output must end in `.png`.
One page uses `sheet.png`; multiple pages use `sheet-001.png`, `sheet-002.png`, and so on.
Each cell shows a timecode on the left and its absolute frame number on the right, in the
built-in monospace font. The page heading gives the frame rate and DF/NDF mode. Timecodes
start at zero: `HH:MM:SS:FF`, with `;FF` for drop-frame at 29.97/59.94 fps. A checkerboard
shows transparent areas. Frames are reduced to thumbnails as they are drawn, and each page is written before the
next is allocated, so full-sized frames do not accumulate in memory.

With `--json`, `files` lists the images and `storyboard.pages` describes each page: `file`,
`width`, `height`, `columns`, `rows` and `cells`. A cell gives `frame`, `time` in seconds,
the displayed `timecode` and its content rectangle (`x`, `y`, `width`, `height`) in that
page's pixels, excluding its label and border. These options and results also apply to
Timeline and Project renders.

## The file

| Export | Required | What it is |
| --- | --- | --- |
| `default` | yes | The root component |
| `composition` | yes | `{ width, height, fps, duration }`: pixels, frames a second, seconds |
| `controls` | no | Props, data and resources a timeline or the command line binds: see below |
| `role` | no | How a timeline uses the scene: see the timeline roles |

`composition`, `controls` and `role` are object literals: tools read them without running the
scene. Built-in functions and tags need no import. A scene may import its own relative
modules (`import { palette } from "./theme.ts"`); npm packages are not available.

## Rules of a frame

- A component is a function of its props. Read the time with `useCurrentTime()` and the
  composition with `useComposition()`. There is no state: `useState`, `useRef`, `useEffect`
  and the like are errors, and so are `fetch`, `Date`, `Math.random` and timers.
- Write time in seconds. Changing `fps` changes how many frames there are, not how fast
  things move.
- Values that depend on the time are traced, not computed once: a branch on the time
  (`seconds < 1 ? a : b`) works, but both of its sides are part of the scene. Give elements
  in the two branches different `key`s.
- Lists come from arrays fixed before the first frame: a module constant, a prop or
  `controls.data`. Map them with a stable `key`, and decide per item what it shows over
  time.
- For randomness use `seededRandom` and `noise1d`/`noise2d`, which are the same everywhere.

`useCurrentTime()` gives `seconds` and `progress` (0 to 1 over the duration), and
`clipSeconds` and `clipProgress` when the scene plays as a timeline clip. `useComposition()`
gives `width`, `height`, `fps`, `duration` and `clipDuration`.

## Elements

| Tag | What it is |
| --- | --- |
| `View` | A box, like a `div`: layout, background, border, children |
| `Text` | A run of text; text must be inside a `Text`. `split` and `perUnit` animate characters, words or lines |
| `Span` | Styled text inside a `Text` |
| `Image` | `src`: a `resource://name`, a relative path or an `https://` address |
| `Video` | `src`, with `trimStart` and `rate`, or `time` for the source time directly |
| `Mask` | `source={<…/>}`, `mode` `alpha` or `luminance`, `invert`: shows its children through another tree |
| `TimeScope` | `offset` and `rate`: a child clock, `(time - offset) × rate` |
| `Echo` | `count`, `delay`, `decay`: copies of its children at earlier times |
| `Shutter` | `samples`, `angle`: motion blur from several sub-frame samples |
| `World` | `camera={{ center, zoom, rotation }}`: a 2D camera over its children |
| `ShaderLayer` | Your own WGSL shader: `code` or `source`, `uniforms`, `inputs`, `outset`; see below |
| `Transition` | Goes between two children over the scene's time |
| `svg` | Inline SVG: `path`, `circle`, `rect`, `line`, `polyline`, `polygon`, `g`, gradients, `text` |

HTML tags (`div`, `span`, `img`, `h1`…) are errors that name the tag to use instead.

## Styles

`style` takes CSS properties in camelCase, as React writes them: `backgroundColor`,
`borderRadius`. Numbers are pixels. `className` takes Tailwind classes (v4), including
arbitrary values such as `w-[340px]`; both can be used together, `style` winning.

Supported, among others:

- Layout: `display` `flex`, `grid`, `block`, `none`; flexbox and grid properties, `gap`,
  `position` `relative`/`absolute` with `inset`, `left`…, sizes with `min-`/`max-`, `margin`,
  `padding`, `aspectRatio`, `overflow: "hidden"`, `zIndex`.
- Paint: colors in every CSS form, `background` with gradients (linear, radial, conic) and
  images, `border`, `borderRadius`, `boxShadow`, `outline`, `opacity`, `mixBlendMode`,
  `clipPath`, `maskImage`.
- Transforms: `transform`, `translate`, `rotate`, `scale`, `transformOrigin`, and 3D ones
  with `perspective`.
- Filters: `filter` and `backdropFilter`: `blur`, `brightness`, `contrast`, `grayscale`,
  `hue-rotate`, `invert`, `saturate`, `sepia`, `drop-shadow`, `opacity`, plus Valle's
  `bloom`, `displacement`, `film-grain`, `chromatic-aberration`, `lens-distortion` and
  `radial-blur`, documented below.
- Text: `fontFamily`, `fontSize`, `fontWeight`, `fontStyle`, `lineHeight`, `letterSpacing`,
  `textAlign`, `textTransform`, `textDecoration`, `textShadow`, `WebkitTextStroke`,
  `fontVariationSettings`, `whiteSpace`.
- `motionBlur: "auto"`: blur along the element's movement.

Not supported: CSS variables (use JavaScript constants), `@keyframes` and CSS transitions
(animate with values over time), `position: fixed`/`sticky`, hover and other states. An
unsupported property or class is an error that says so.

## Valle filters

Use these in `style.filter` for the element and its children, or `style.backdropFilter`
for the picture behind its box. Functions run from left to right; separate functions and
their arguments with spaces, not commas. These six functions are Valle extensions, not
browser CSS.

Lengths below take `px`, such as `2px`, not `2`, `2em` or `20%`. A unitless `0` is accepted
where the range includes zero. Lengths are layout pixels and scale with the output size.
Numbers have no unit. Square brackets mark optional arguments; do not write the brackets.

| Function | Parameters | Effect |
| --- | --- | --- |
| `bloom(radius intensity threshold [knee])` | `radius`: 0–128px; `intensity`: 0–4; `threshold`: 0–1; `knee`: 0–1, default 0.1 | Adds a blurred glow from bright areas. `threshold` selects the brightness; `knee` softens that cutoff. |
| `displacement(seed fx fy scale [octaves [mode]])` | `fx`, `fy`: 0–1 cycles per layout pixel; `scale`: −256–256px; `octaves`: integer 1–4, default 2; `mode`: `fractal` (default) or `turbulence` | Warps the picture with seeded noise. Higher frequencies make smaller ripples; more octaves add detail. Supply `octaves` before `mode`. |
| `film-grain(seed amount size)` | `amount`: 0–1; `size`: 1–64px | Adds seeded grain to colors, preserving alpha. `size` is the noise cell size. |
| `chromatic-aberration(offset)` | `offset`: −256–256px | Moves red and blue in opposite horizontal directions, each by half the offset; green stays put. |
| `lens-distortion(k1 k2)` | `k1`, `k2`: each −0.5–0.5 | Warps around the box center, with quadratic and quartic radial coefficients. Zero leaves the picture unchanged. |
| `radial-blur(x y amount)` | `x`, `y`: −1000000–1000000px from the border box's top left; `amount`: 0–128px | Blurs along rays from `(x, y)`, by up to `amount` in each direction. |

Both `seed` parameters are integers from 0 to 4294967295. A fixed seed gives a fixed pattern;
the noise does not animate by itself. All parameters are required unless marked optional.

```tsx scene filters.motion.tsx
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

const effects = [
  "bloom(12px 1 0.6)",
  "displacement(7 0.03 0.03 12px 2 turbulence)",
  "film-grain(3 0.4 2px)",
  "chromatic-aberration(4px)",
  "lens-distortion(0.2 0)",
  "radial-blur(90px 70px 8px)",
];

export default function Filters() {
  return (
    <View style={{
      display: "flex", width: 640, height: 360, padding: 24, gap: 24,
      flexDirection: "row", flexWrap: "wrap", background: "#182030",
    }}>
      {effects.map((filter) => (
        <View key={filter} style={{
          display: "flex", width: 180, height: 140,
          alignItems: "center", justifyContent: "center",
          background: "linear-gradient(135deg, #245080, #b04070)", filter,
        }}>
          <Text style={{ fontSize: 48, fontWeight: 700, color: "white" }}>Valle</Text>
        </View>
      ))}
    </View>
  );
}
```

To combine effects, write `filter: "bloom(12px 1 0.6) film-grain(3 0.2 2px)"`. Animate
parameters with template strings, for example ``filter: `bloom(${radius}px 1 0.6)` ``.

## Custom WGSL shaders

`ShaderLayer` draws a box with your WGSL function. It can read its children as a texture,
the backdrop, or supplied images; a procedural shader can also draw without children.
Give it a size through `style` or layout. This complete scene gradually inverts its children:

```tsx scene shader.motion.tsx
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

const code = `
struct Params { amount: f32 }
@group(2) @binding(0) var<uniform> params: Params;
@group(2) @binding(1) var content: texture_2d<f32>;
@group(2) @binding(2) var sampling: sampler;

fn shade(input: ShaderInput) -> vec4f {
  let color = textureSampleLevel(content, sampling, input.uv, 0.0);
  let inverted = mix(color.rgb, vec3f(1.0) - color.rgb, params.amount);
  return vec4f(inverted, color.a);
}`;

export default function Shader() {
  const { seconds } = useCurrentTime();
  const amount = interpolate(seconds, [0, 2], [0, 1]);
  return (
    <ShaderLayer code={code} uniforms={{ amount }} style={{ width: 640, height: 360 }}>
      <View style={{
        display: "flex", width: "100%", height: "100%",
        alignItems: "center", justifyContent: "center",
        background: "linear-gradient(135deg, #245080, #b04070)",
      }}>
        <Text style={{ fontSize: 80, fontWeight: 700, color: "white" }}>WGSL</Text>
      </View>
    </ShaderLayer>
  );
}
```

The entry is `fn shade(input: ShaderInput) -> vec4f`. Valle supplies `ShaderInput` and the
fragment entry: do not redeclare the struct or add `@fragment`, `@vertex` or `@compute`.
Return sRGB-encoded RGB and straight (unpremultiplied) alpha. Valle clamps the four channels
to 0–1, applies edge coverage and premultiplies for compositing; do not multiply RGB by
alpha yourself. Input textures also contain sRGB colors with straight alpha.

| `ShaderInput` field | Type | Meaning |
| --- | --- | --- |
| `uv` | `vec2f` | Normalized local coordinates, `(0, 0)` at the bottom left and `(1, 1)` at the top right. Input textures use the same orientation. |
| `size` | `vec2f` | The box's width and height in layout pixels, before transforms and output scaling. |
| `pixel` | `vec2f` | `uv` multiplied by the box's raster dimensions, which account for rendering scale. A local pixel coordinate, not a pixel size or a position on the whole canvas. |

Use `input.uv * input.size` for positions in layout pixels; `input.pixel` is useful for
patterns in raster pixels. With `outset`, coordinates outside the original box can go
below 0 or above 1; the UV range is not stretched to include the extra area.

| Prop | Meaning |
| --- | --- |
| `code` | A fixed WGSL string, as above. |
| `source` | A shader resource such as `"resource://wave"`, instead of `code`. Declare it with `resource({ kind: "shader", default: "./wave.wgsl" })` in `controls.resources`. |
| `uniforms` | Values by uniform **field** name, such as `{ amount: seconds }`, not by the struct or variable name. Values can animate; code and bindings stay fixed. Omitted fields are zero. |
| `inputs` | Textures by WGSL variable name, such as `{ picture: "resource://photo" }` for `var picture: texture_2d<f32>`. Declare the image resource in `controls.resources`; Timeline roles can also use `"input://source"`, `"input://from"` or `"input://to"` where available. |
| `outset` | A fixed nonnegative number of layout pixels to paint past each side of the box, default 0. It does not change layout or enlarge the captured input textures; ancestor clipping still applies. |

Author bindings use `@group(2)`; groups 0 and 1 belong to Valle. Binding numbers must be
unique but need not be consecutive. You may declare one uniform struct, `texture_2d<f32>`
images and filtering samplers. Uniform fields support `f32`, `i32`, `u32`, vectors,
matrices and fixed arrays. Pass vectors, matrices and arrays as flat JavaScript arrays
(matrices column by column); Valle handles WGSL padding. Color strings can fill `vec3f`
or `vec4f` fields. Use a `u32` field for a JavaScript boolean, not WGSL `bool`.

Texture variable names select what to read: `content` is the layer's content, including its
children; `backdrop` is the picture behind the layer, captured over its box. Declare either
only when needed, without adding it to `inputs`. Other texture names must match `inputs`
keys. Use `textureDimensions(texture)` when you need a supplied image's dimensions: they
can differ from the layer's size. Time is not an implicit shader global; pass `seconds`
through `uniforms` when needed.

## Animation

Animate by computing values from the time:

```tsx
const { seconds } = useCurrentTime();
const x = interpolate(seconds, [0, 1, 2], [0, 400, 300], { easing: "ease-in-out" });
const tint = interpolate(seconds, [0, 2], ["#22d3ee", "#a855f7"], { colorSpace: "oklch" });
const pop = spring(seconds - 0.5, { preset: "bouncy" });
```

- `interpolate(t, inputs, outputs, options)` maps rising `inputs` to `outputs`: numbers,
  colors, lengths (`"24px"`) or angles (`"90deg"`). Options: `easing` (CSS easing or a named
  preset below), `colorSpace`, `hue`, `extrapolateLeft` and
  `extrapolateRight` (`clamp`, the default, or `extend`).
- `spring(t, options)` goes from 0 to 1 like a spring: `preset`, or `duration` and
  `bounce`, or `mass`, `stiffness`, `damping`; `springVelocity` gives its speed.
- `defineSequence({ name: { at, duration } | { after, overlap, delay, duration } })` lays out
  stages in time; `stageProgress(t, stage)` and `staggerProgress(t, stage, index, gap)` give
  their 0 to 1 progress.
- `wiggle(t, { seed, frequency, amplitude })`, `wiggle2D`, `noise1d`, `noise2d`,
  `seededRandom(seed, count, [low, high])` for organic movement.
- `freezeTime`, `pingPong`, `repeatProgress`, `yoyoProgress`, `clamp`, `mod` reshape time.
- `<Text split="char" perUnit={(unit) => style}>` (or `"word"`, `"line"`) with
  `rangeSelector(unit, { start, end, softness, shape })` animates text unit by unit; a split
  `Text` holds plain text, no `Span`s.

For movement, animate `transform` or `translate`, and keep the layout still: animating
`left`, `top` or margins moves the layout itself, which is slower and can step.

### Easing presets

`easing` accepts CSS keywords such as `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`,
`step-start` and `step-end`, as well as `cubic-bezier(…)`, `steps(…)` and `linear(…)`.
It also accepts these 33 case-sensitive preset names:

- `easeIn`, `easeOut`, `easeInOut`: aliases for `ease-in`, `ease-out`, `ease-in-out`.
- `easeIn` / `easeOut` / `easeInOut` followed by any family below, such as `easeOutCubic`
  or `easeInOutElastic`. `In` eases into movement, `Out` eases into the end, and `InOut`
  eases both ends.

| Family | Motion |
| --- | --- |
| `Sine` | Gentle acceleration and deceleration |
| `Quad`, `Cubic`, `Quart`, `Quint` | Progressively stronger acceleration and deceleration |
| `Expo` | An abrupt start or finish to the change in speed |
| `Circ` | Circular-arc-style acceleration and deceleration |
| `Back` | Pulls back before starting, or overshoots before settling |
| `Elastic` | Oscillates around the endpoint |
| `Bounce` | Bounces toward the endpoint |

```tsx
const x = interpolate(seconds, [0, 1], [0, 400], { easing: "easeOutCubic" });
```

`easeOutCubic` is the same preset as `cubic-bezier(0.33, 1, 0.68, 1)`. These names select
fixed approximations, not exact evaluations of the named mathematical functions: the
first eight families use cubic Béziers, and `Elastic`/`Bounce` use `linear()` curves with
41 evenly spaced samples rounded to three decimal places. Back and Elastic can go below
0 or above 1. Presets are resolved when parsed, not looked up each frame. Timeline keyframes
accept the same names, including audio gain and pan automation.

## Text and fonts

Built-in fonts, used without declaring them: Noto Sans (the default, and `sans-serif`), Noto
Serif (`serif`), Noto Sans Mono (`monospace`), Noto Sans SC and KR for Chinese and Korean, and
Noto Color Emoji. They are downloaded once into the cache on first use.

Other fonts are resources of kind `font`, used by their name in `fontFamily`:

```tsx
export const controls = {
  resources: {
    brand: resource({ kind: "font", default: ["fonts/Inter-Regular.ttf", "fonts/Inter-Bold.ttf"] }),
  },
};
// style={{ fontFamily: "brand", fontWeight: 700 }}
```

`measureText` and `textOutline` measure text and give its outline as a path.

## Controls

`controls` declares what can be bound from outside: by `--props`, `--data` and `--resource`
on the command line, by a timeline clip, and by Studio's form.

```tsx scene card.motion.tsx
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export const controls = {
  props: {
    title: string({ default: "Quarterly results", label: "Title" }),
    accent: color({ default: "#f97316" }),
    size: number({ default: 64, min: 12, max: 160, step: 1 }),
  },
  data: {
    rows: { default: [{ label: "A", value: 3 }, { label: "B", value: 5 }] },
  },
};

export default function Card({ title, accent, size, rows }) {
  return (
    <View style={{ width: 640, height: 360, padding: 32, backgroundColor: "#0f172a", gap: 12 }}>
      <Text style={{ fontSize: size, color: accent }}>{title}</Text>
      {rows.map((row) => (
        <Text key={row.label} style={{ fontSize: 28, color: "white" }}>
          {row.label}: {row.value}
        </Text>
      ))}
    </View>
  );
}
```

- Prop kinds: `number`, `string`, `boolean`, `color`, `select({ values })`, `point`,
  `length`, `angle`, `rect`. Each takes `default`, `label`, and `required` for one without a
  default.
- `data` entries take any JSON default, such as a chart's rows. The root component receives
  props and data as its props.
- `resources` entries are `resource({ kind, default })`, kinds `image`, `video`, `font`,
  `shader`, read as `resource://name` (a font by its name in `fontFamily`).

```sh
valle motion render card.motion.tsx --props '{"title":"Q3"}' --data @rows.json -o card.png
valle motion render scene.motion.tsx --resource photo=me.jpg -o out.mp4
```

## Shapes, paths and charts

- Inline `<svg>` draws shapes; animate their attributes with values over time.
- Path functions return path data for `<path d={…}>`: `pathLength`, `pointAt`, `tangentAt`,
  `roundCorners`, `pathBoolean(a, b, op)`, `inflatePath`, `morph(from, to, progress)`,
  `zigzag`, `twist`, `puckerBloat`, `connect(a, b)`.
- `bounds` and `anchor` read where an element was laid out, for lines that connect boxes;
  `flip` and `defineLayoutStates` animate between layouts.
- D3-style charts, without importing D3: `scaleLinear`, `scaleBand`, `scalePoint`,
  `scaleSequential`, `scaleQuantize`, `ticks`, `nice`, `extent`, `line`, `area`, `arc`,
  `pie`, `stack`, `curveLinear`, `curveMonotoneX`, `geoPath` with `geoMercator` and
  `geoAlbers`, `placeLabels`, `graphLayout`.
- `particles(t, options)` gives 2D particles to map into elements; `field(…)` moves a group
  from one arrangement to another with a stagger.
- `formatNumber(v, { decimals, grouping })`, `formatPercent` and `padNumber(v, { width })`
  format numbers the same everywhere.

## Timeline roles

A scene declares how a timeline uses it:

```tsx
export const role = overlay({ intro: 0.5, outro: 0.5, hold: "once" });
```

| Role | Used as | What it reads |
| --- | --- | --- |
| none | a `motion` clip, played as it is | |
| `overlay({ intro, outro, hold })` | a title or lower third of any length | its intro and outro keep their length; `hold` is `once`, `loop` or `stretch` |
| `transition()` | a timeline `motion` transition | `<Image src="input://from">` and `input://to` |
| `effect()` | a clip's `effects`, or an adjustment clip | `input://source`: the clip, or all under it |
| `captionPresenter({ intro, outro })` | a caption track's `presenter` | its `caption` prop: `{ text, runs }` |

A transition or an effect fills its composition with what it reads and draws over it; use
`clipProgress` for how far it is.

## Not in this version

These are errors that say they come in a later version: `Scene3D` and its 3D elements,
`Glass` and `GlassField`, `MathFormula`, and the functions `particles3d`, `project3d`,
`extrude`, `lathe`, `tube`, `morphSequence`, `noiseDisplace`, `simplify`, `resamplePath`,
`reversePath`, `strokeToPath`, `pathTrajectory`, `fitText`. CSS 3D transforms work.

## Errors

`check` reports each problem once, with the frames it is on, and where it is in the file:

```text
frame 0 and 29 more: title.motion.tsx:4:5: TypeError: HTML tags are not supported: use <View> instead of <div>
error: 1 problem in 30 frames
```

The message usually says what to write instead: do that, then check again.
