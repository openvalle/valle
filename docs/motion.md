# Motion authoring reference

Motion turns a `.motion.tsx` file into a video scene. This reference describes the
authoring surface in this repository: JSX, styles, animation, data, resources,
geometry and effects. Use the [CLI guide](cli.md) for command output contracts,
installation and the other Valle command groups.

Motion compiles a constrained, deterministic JSX language. Its component tree is
prepared before rendering; expressions then supply values for each requested
frame. Components and builtins below are available without imports. React hooks,
the browser DOM and arbitrary npm packages are not part of this environment.

## Contents

- [First scene](#first-scene)
- [Language and components](#language-and-components)
- [Controls, data and assets](#controls-data-and-assets)
- [Time and animation](#time-and-animation)
- [CSS and layout](#css-and-layout)
- [Tailwind class catalog](#tailwind-class-catalog)
- [Text, fonts and formulas](#text-fonts-and-formulas)
- [Paths, shapes and paint](#paths-shapes-and-paint)
- [Clipping, masks and effects](#clipping-masks-and-effects)
- [Data visualization and repeated geometry](#data-visualization-and-repeated-geometry)
- [Camera, 3D and shaders](#camera-3d-and-shaders)
- [Troubleshooting and limits](#troubleshooting-and-limits)
- [Examples and verification](#examples-and-verification)

## First scene

Save this complete file as `title.motion.tsx`:

```tsx
export default function Title(ctx) {
  const reveal = interpolate(ctx.progress, [0, 0.25], [0, 1], { easing: "easeOut" });
  return (
    <Scene className="relative h-full w-full flex items-center justify-center"
      style={{ backgroundColor: "#102030" }}>
      <Text style={{ fontSize: 48, fontWeight: 700, color: "#ffffff",
        opacity: reveal, translate: point(0, (1 - reveal) * 24) }}>
        Hello, Valle!
      </Text>
    </Scene>
  );
}
```

With `valle` on PATH (use `./dist/bin/valle` in a source checkout):

```sh
valle motion check title.motion.tsx --size 640x360 --json
valle motion render title.motion.tsx --duration 3 --fps 30 --size 640x360 --frame 30 -o title.png --backend raster --json
valle motion studio title.motion.tsx --duration 3 --fps 30 --size 640x360
valle motion render title.motion.tsx --duration 3 --fps 30 --size 640x360 -o title.mp4 --events
```

`--frame` is zero-based and selects PNG output; without it, render produces MP4.
Destinations must be new files. Pure graphics PNG rendering needs no FFmpeg; MP4
encoding and video decoding need compatible FFmpeg shared libraries. See
[runtime setup](cli.md#runtime-notes).

`--size` defines the logical canvas used by layout and `ctx.viewport`.
`--output-size` scales delivery without changing layout. Keep duration, FPS, canvas,
data, assets and fonts consistent between previews and export. Defaults are five
seconds, 30 FPS and 1920×1080; rational FPS such as `30000/1001` is accepted.

## Language and components

### Module and function shape

The entry module needs a default function declaration. Name it, or supply
`export const component = "name"`. Optional `export const controls =
defineControls(...)` declares external inputs. The entry module's named exports
are `component` and `controls`; helper modules may export reusable values and
components.

The root parameter positions are `(ctx, props, signals, data)`. Omit unused trailing
parameters. `props` and `data` may be destructured in their positions. An authored
child component uses `(ctx, props, signals)`; pass prepared data through its props.
The first parameter is the Motion context, including in child components.

Module declarations use `const` and functions. Component bodies use `const`
bindings followed by a returned JSX expression. Function and arrow helpers can
return scalars or JSX. Pure prepare-time helpers can perform bounded data work;
frame-dependent helper bodies must lower to supported expressions. Do not use
async functions, generators, recursive components/helpers, mutation or loops to
construct a new tree on every frame.

| Construct | Contract |
| --- | --- |
| Numbers, strings, booleans, template literals | Finite numbers; template holes must have supported value types |
| Arithmetic and comparisons | `+`, `-`, `*`, `/`, `%`, unary signs, equality and ordered comparisons; operands must type-check |
| Conditional values | `condition ? a : b`, boolean expressions; branches need compatible types |
| JSX conditions | Static conditions can choose a subtree; frame-dependent conditions retain prepared branches and gate visibility |
| Lists | Map a prepare-time array with a synchronous arrow callback; each returned node/component needs a stable `key` |
| Arrays and tuples | Fixed shape; supported typed constructors and bounded maps; runtime indexing has explicit bounds/type checks |
| Destructuring | Explicit object fields and array entries, including supported defaults; no rest or computed field names |
| TypeScript | `as const`, `satisfies`, assertions and non-null wrappers are peeled where supported; this is not a TypeScript type checker |
| JSX fragments | Use a `Group` or `View` to give multiple children one root |
| Spread | No JSX prop spread or style/per-unit spread; list and interpolation arrays cannot have spreads or holes |

Prepare-time means independent of `ctx`, runtime `props` and cue signals. Module
constants, bound `data`, static component arguments, a theme and explicit resources
can participate in preparation. `props` have typed runtime bindings even when a
default exists; use `data` for a collection that determines node count.

### Reusable components, children and themes

Save as `cards.motion.tsx`:

```tsx
const cards = [
  { id: "write", title: "Write" },
  { id: "preview", title: "Preview" },
  { id: "render", title: "Render" },
];
const theme = { background: "#0f172a", card: "#1e293b", accent: "#38bdf8" };

function Card(ctx, { title, index, children }) {
  const palette = useTheme();
  const reveal = clamp(ctx.progress * 4 - index * 0.2, 0, 1);
  return (
    <View key="body" className="flex flex-col" style={{ width: 176, padding: 20,
      borderRadius: 16, backgroundColor: palette.card, opacity: reveal }}>
      <Text key="title" style={{ fontSize: 24, fontWeight: 700, color: palette.accent }}>{title}</Text>
      {children}
    </View>
  );
}

export default function Cards(ctx) {
  return (
    <ThemeProvider value={theme}>
      <Scene className="h-full w-full flex items-center justify-center gap-4"
        style={{ backgroundColor: useTheme().background }}>
        {cards.map((card, index) => (
          <Card key={card.id} title={card.title} index={index}>
            <Text key={`${card.id}-caption`} style={{ marginTop: 12, fontSize: 16, color: "#cbd5e1" }}>One scene.</Text>
          </Card>
        ))}
      </Scene>
    </ThemeProvider>
  );
}
```

`ThemeProvider` is a prepare-time scope, with one JSX root; it adds no rendered box.
Its value must be static, serializable theme data. `useTheme()` reads the nearest
provider and fails outside one. A nested provider supplies its own value; explicitly
compose any desired inheritance in the prepared object. These two intrinsic names
cannot be shadowed. Children passed to a component are JSX nodes; wrap prose in
`Text`. Pass JSX through children rather than a JSX-valued prop.

Use stable semantic IDs for data lists. An index-derived key is suitable for a
fixed authored list; a data item ID preserves its identity across reordering.
Keys must be non-empty and unique in their expanded scope. The current compiler
requires explicit keys on list roots; static descendants derive stable paths
from their keyed parent. Component expansion scopes
descendant keys to the component instance, so a reusable component can use fixed
internal keys such as `body` and `title`.

### Local modules

Use explicit relative imports, for example `import { Card } from
"./components/card"`. Resolution stays within the supplied source directory tree;
use `.ts`/`.tsx` files and unambiguous relative paths. Explicit named imports and
supported default imports are linked before compilation. Type-only imports do
not create runtime dependencies.

No npm/bare imports, URLs, absolute imports, dynamic `import()`, side-effect imports,
namespace imports or module cycles. Filesystem/network work belongs outside the
Motion source. Bind JSON using `--data`, and media using `--asset`.

### Primitive catalog

All ordinary primitives accept `key`, static `className`, literal-object `style`
and `visible={booleanExpression}`, subject to the component-specific restrictions
below. Unknown attributes and event handlers are rejected.

| Primitive | Purpose and additional attributes |
| --- | --- |
| `Scene` | Composition root; optional 2D `camera` |
| `Group` | Group children; shares the layout model, does not imply absolute positioning |
| `View` | General layout/paint box |
| `Flex`, `Absolute` | Boxes with implied `flex` or `absolute` class |
| `World`, `Screen` | Camera coordinate spaces; see [camera](#camera-3d-and-shaders) |
| `Text` | Text, numeric/string expressions, direct `Span` and inline `Image` children; `split`, `perUnit`, `path` |
| `Span` | Direct child of `Text`; restricted inline `style` only |
| `Image` | Static `src` referencing a bound image asset |
| `Video` | Static `src` referencing a video asset; `sourceStart`, `speed` |
| `Path` | Typed `d`, fill/stroke and path reveal attributes |
| `Circle`, `Ellipse`, `Rect`, `Line`, `Polyline`, `Polygon` | Shape coordinates plus Path paint attributes |
| `GeometryBatch` | Repeated circles/rectangles; `geometry`, `positions`, `sizes`, `fills`, optional `opacities`, `semanticKeys` |
| `Clip` | Path-clipped children; `path` (or `d`), `fillRule` |
| `Mask`, `MaskSource` | Alpha/luminance masking; see [masks](#clipping-masks-and-effects) |
| `MathFormula` | Formula leaf; static `latex`, `displayMode`, `ariaLabel` |
| `Glass`, `GlassField` | Optical material surfaces and shared material fields |
| `ShaderLayer` | Package-backed custom shader; `source`, `inputs`, `uniforms`, content children |
| `Scene3D` | 3D scene leaf with its own camera and explicit 3D children |
| `ThemeProvider` | Compile-time theme scope; `value` and one root child |

HTML aliases are deliberately small: `div`, `section`, `main`, `article`, `svg`
map to `View`; `span`, `p`, `strong`, `h1`, `h2`, `h3` map to `Text`; `img` and
`video` map to their corresponding media primitives. Lowercase SVG shape names and
`path` are also admitted. These aliases do not establish browser element defaults:
set heading size and weight explicitly. `svg` is a box alias, not a full SVG DOM
with `viewBox`, `<defs>`, CSS selectors or arbitrary SVG attributes.

## Controls, data and assets

### Controls namespaces

| Namespace | Declaration and use |
| --- | --- |
| `props` | Typed scalar/geometric controls read through `props.name` |
| `data` | Validated structured JSON available during prepare through the fourth root argument |
| `timing` | `enterFrames: frames(...)`, `exitFrames: frames(...)`, `holdCycleFrames: optionalFrames(...)` |
| `cues` | Named `spanCue({ required })` declarations, read through `signals.name` |
| `assets` | `asset({ kind, required })`, referenced by `asset://name` |
| `camera` | Typed camera controls for host integration; the scene's authored camera is `Scene camera={{...}}` |

Prop constructors: `number`, `string`, `boolean`, `color`, `length`, `angle`,
`point`, `rect`, `path`, `nodeTarget`, `select`. Common options are `default`,
`required`, `label`; numbers add `min`, `max`, `step`; `select` requires a string
`values` array. Examples: `number({ default: 24, min: 0 })`,
`select({ default: "light", values: ["light", "dark"] })`,
`point({ default: point(40, 60) })`.

The constructors `point`, `rect`, `path`, `boolean` and `frames` are overloaded:
their schema forms differ from geometry/sequence forms described later.
Length and angle defaults are strings such as `"24px"` and `"15deg"`.

Use `--props props.json` for a JSON object of constant prop values and
`--cues cues.json` for a JSON object of Timeline cue bindings. These options work
with `check`, `render` and `studio`, alongside `--asset`, `--data` and `--font`.
Required inputs without defaults must be supplied. Missing optional cues stay
inactive; declaring a cue does not schedule narration automatically.

For example, `cues.json` can contain:

```json
{ "reveal": { "type": "source-range", "start": 0, "end": 2.5 } }
```

Cue times are seconds in the Motion source. See [Timeline integration](timeline.md#motion-integration)
for animated prop curves and clip placement. `motion check` uses the same preparation
and Native Raster rendering path for one frame (default `--frame 0`); pass the same
`--duration`, `--fps`, `--size` and bindings as the intended render. It creates no
persistent output and does not prove that every frame or encoder will succeed.

### Structured data example

Save as `bars.motion.tsx`:

```tsx
export const controls = defineControls({
  data: {
    rows: array(record({
      id: string(), label: string(), value: number({ min: 0, max: 100 }),
    }), { minItems: 1, maxItems: 5, key: "id" }),
  },
});

export default function Bars(ctx, props, signals, data) {
  const reveal = interpolate(ctx.progress, [0, 0.5], [0, 1]);
  return (
    <Scene className="h-full w-full flex flex-col justify-center"
      style={{ padding: 40, gap: 16, backgroundColor: "#0f172a" }}>
      {data.rows.map((row) => (
        <View key={row.id} className="flex items-center" style={{ gap: 16 }}>
          <Text key={`${row.id}-label`} style={{ width: 90, fontSize: 20, color: "#e2e8f0" }}>{row.label}</Text>
          <View key={`${row.id}-bar`} style={{ width: row.value * 3 * reveal, height: 28,
            borderRadius: 6, backgroundColor: "#38bdf8" }} />
        </View>
      ))}
    </Scene>
  );
}
```

Save `bars.json`:

```json
{
  "rows": [
    { "id": "a", "label": "Alpha", "value": 80 },
    { "id": "b", "label": "Beta", "value": 55 },
    { "id": "c", "label": "Gamma", "value": 95 }
  ]
}
```

```sh
valle motion check bars.motion.tsx --data bars.json --size 640x360
valle motion render bars.motion.tsx --data bars.json --size 640x360 --duration 3 --fps 30 --frame 45 -o bars.png
```

Data schemas accept `number({ min, max })`, `string({ minBytes, maxBytes })`,
`boolean()`, `color()`, `point()`, `record({ fields... })`, `tuple([schemas...])`,
and `array(itemSchema, { minItems, maxItems, key })`. Each array needs `maxItems`;
`key` names an item field for stable identity. Nested arrays need their own bounds.
The input is the direct object matching `controls.data`, without an extra `data`
wrapper. A data change requires preparation again; Studio watches the bound JSON.

### Images, video and fonts

Save as `poster.motion.tsx` and provide your own `poster.png`:

```tsx
export const controls = defineControls({
  assets: { poster: asset({ kind: "image", required: true }) },
});
export default function Poster(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0f172a" }}>
      <Image src="asset://poster" style={{ position: "absolute", left: 32, top: 32,
        width: 576, height: 296, objectFit: "cover", borderRadius: 20 }} />
    </Scene>
  );
}
```

```sh
valle motion check poster.motion.tsx --asset poster=poster.png --size 640x360
valle motion render poster.motion.tsx --asset poster=poster.png --size 640x360 --frame 0 -o poster-frame.png
```

Media primitives default to block layout; use explicit `display: "inline"` when
placing an image in a text line. Asset kinds are `image`, `audio`, `video`, `font`,
`model3d`. References are static;
the declared kind must match the consumer. Required assets must be bound. Do not
put HTTP/file paths directly in `src` or use CSS `url(...)` as a resource loader.

`Video` uses `sourceStart` in seconds (default 0) and `speed` (default 1). Media
sampling still follows the local Motion clock; `visible` hides drawing without
moving the source clock. A `Video` node contributes video pixels, not an audio
track. Use Timeline audio clips for sound; an `Audio` JSX primitive is not exposed.

Both attributes accept finite numbers or frame expressions; they are not limited
to 0–1. For example, `sourceStart={6} speed={2}` starts at source second 6 and
samples source second 8 after one local second. The source time is
`max(0, sourceStart + localSeconds * speed)`; `speed={0}` holds the source start.

For a portable project font, declare `brand: asset({ kind: "font", required: true })`,
bind `--asset brand=fonts/Brand.ttf`, and set `fontFamily: "asset://brand"`. An extra
`--font fonts/Brand.ttf` instead registers that file by its font family for render
and Studio. Supply the same `--font` options to `motion check`.

## Time and animation

### Context fields

| Expression | Meaning |
| --- | --- |
| `ctx.localFrame` | Zero-based integer frame in the local clip |
| `ctx.seconds` | Continuous local sample time in seconds |
| `ctx.progress` | Local time / clip duration, in `[0, 1)` |
| `ctx.durationFrames` | Clip duration in frames |
| `ctx.fps.num`, `ctx.fps.den` | Rational frame-rate numerator/denominator |
| `ctx.viewport.width`, `.height` | Logical canvas dimensions in pixels |
| `ctx.enter`, `ctx.hold`, `ctx.exit` | Phase fields listed below |
| `ctx.unit.index`, `.count`, `.start`, `.end` | Only inside `Text perUnit`; start/end are text byte offsets |

Each phase exposes `active`, `frame`, `elapsedFrames`, `durationFrames`, `progress`.
`frame` and `progress` clamp to the phase window. `elapsedFrames` is unclamped,
negative before a phase and increasing after it ends. Hold additionally exposes
`iteration`, `cycleFrame`, `cycleProgress`.

These are the author-readable fields; internal context serialization also contains
fields that are not exposed as expressions. Test phases with `ctx.enter.active`
etc., rather than assuming `ctx.currentPhase` is available.

With no timing declaration, enter/exit lengths are zero and hold fills the clip.
Otherwise hold receives `duration − enter − exit`. If the clip is shorter than
enter plus exit, those windows compress proportionally and hold becomes empty.
Progress is `frame / windowLength`: the last active frame normally does not equal
1. Entry reaches 1 after its window; an exit ending with the clip has no later
frame. End keyframes earlier if the final displayed frame must show a settled state.

### Interpolation and springs

`interpolate(input, inputRange, outputRange, { easing }?)` clamps outside its
input range. Ranges must have equal length ≥ 2, strictly increasing finite input
stops, and compatible output types. Stops are known at compile time: literals or
immutable constant arrays. No `ctx`, runtime props, spreads or holes in the stops.

Numbers, colors, lengths, angles and supported geometric values interpolate;
boolean/string/enum values are discrete. Keep corresponding length/angle units
compatible. For a destination depending on a prop or viewport, interpolate a
normalized number and multiply afterward.

Easing names: `linear` (default), `ease`, `easeIn`, `easeOut`, `easeInOut`, `exp`,
and `cubic-bezier(x1,y1,x2,y2)` with x coordinates in `[0,1]`. `easing` can be one
string or an array with one entry per segment. Other option names, including
`extrapolateLeft`/`extrapolateRight`, are not author options in this CLI language.

Save as `spring.motion.tsx`:

```tsx
export const controls = defineControls({
  timing: { enterFrames: frames({ default: 18, min: 0 }),
    exitFrames: frames({ default: 12, min: 0 }) },
});
export default function SpringTitle(ctx) {
  const enter = spring({ elapsedFrames: ctx.enter.elapsedFrames, fps: ctx.fps, preset: "gentle" });
  const exit = interpolate(ctx.exit.progress, [0, 0.8], [1, 0]);
  return (
    <Scene className="h-full w-full flex items-center justify-center"
      style={{ backgroundColor: "#0f172a" }}>
      <Text style={{ fontSize: 44, color: "#f8fafc", opacity: clamp(enter, 0, 1) * exit,
        translate: point(0, (1 - enter) * 50) }}>Spring into motion</Text>
    </Scene>
  );
}
```

`spring({ elapsedFrames, fps: ctx.fps, preset })` starts at 0 with zero velocity
and converges to 1; it can overshoot. It continues settling after a phase when
given `elapsedFrames`. Presets: `gentle`, `wobbly`, `stiff`, `slow`, `bouncy`.
Alternatively provide static `mass`, `stiffness`, `damping` (defaults 1, 100, 10).
Mass/stiffness must be positive; damping must be non-negative. A preset and physical
parameters are mutually exclusive. There are no `from`, `to`, `duration` or
`overshootClamping` options: scale/offset/clamp the result explicitly.

### Sequences and repetition

Save as `sequence.motion.tsx`:

```tsx
const sequence = defineSequence({
  title: stage({ duration: seconds(0.6) }),
  items: stage({ after: "title", overlap: seconds(0.15), duration: seconds(0.8) }),
});
const labels = ["Prepare", "Preview", "Export"];
export default function Sequence(ctx) {
  const title = stageProgress(ctx.localFrame, ctx.fps, sequence.title);
  return (
    <Scene className="h-full w-full flex flex-col items-center justify-center"
      style={{ gap: 18, backgroundColor: "#0f172a" }}>
      <Text style={{ fontSize: 36, color: "#ffffff", opacity: title }}>A clear sequence</Text>
      {labels.map((label, index) => (
        <Text key={label} style={{ fontSize: 24, color: "#38bdf8",
          opacity: staggerProgress(ctx.localFrame, ctx.fps, sequence.items, index, seconds(0.15)) }}>
          {label}
        </Text>
      ))}
    </Scene>
  );
}
```

`stage({ duration, at?, after?, delay?, overlap? })` declares a positive duration.
The first stage defaults to zero; later stages need `at` or an earlier `after`
label. `at` and `after` are exclusive, as are `delay` and `overlap`; delay/overlap
require `after`. A sequence uses either `seconds(...)` or `frames(...)` throughout.
Sequence stages do not extend the render duration automatically.

| Helper | Contract |
| --- | --- |
| `stageProgress(frame, ctx.fps, stage)` | Clamped progress through one named stage |
| `staggerProgress(frame, ctx.fps, stage, index, gap)` | Shift the stage start by index × gap; gap uses the sequence's units |
| `repeatProgress(frame, ctx.fps, stage, count)` | Repeat within the stage; positive static integer count |
| `yoyoProgress(frame, ctx.fps, stage, count)` | Repeated forward/backward progress within the stage |
| `freezeFrame(frame, { from, to })` | Hold at `from` during `[from,to)`, then subtract the frozen interval; static integers `0 ≤ from < to` |
| `defineRepeater({ count, keyPrefix? })` | Prepared list of `{ index, count, progress, key }`; count 1–2048 |
| `trail(progress, index, { gap, mode? })` | Offset normalized progress; mode `clamp` (default) or `wrap` |
| `wiggle(frame, ctx.fps, { seed, frequency, amplitude, phase? })` | Seeded scalar displacement; frequency in seconds, non-negative amplitude |
| `wiggle2D(frame, ctx.fps, { seed, frequency, amplitude: [x,y], phase? })` | Seeded point displacement; suitable for `style.translate` |

Cue signals expose `active`, `progress`, `enter`, `hold`, `exit`, `localFrame`.
Their windows come from host-supplied cue data. Use `visible={signals.name.active}`
or animate with the numeric fields once those cues are bound.

### Deterministic math and formatting

Frame expressions expose `sin`, `cos`, `exp`, `floor`, `ceil`, `round`, `fract`,
`atan2`, `pow`, `mod`, `pingPong`, `clamp`; `Math.sqrt`, `.exp`, `.sin`, `.cos`,
`.tan`, `.atan2`, `.pow`, `.floor`, `.ceil`, `.round`, `.trunc`, `.min`, `.max`,
`.abs` have admitted deterministic lowering. This is a finite list, not all of Math.

Trigonometry uses radians. `TAU` is 2π; `deg(n)` converts degrees to a numeric
radian value and `rad(n)` keeps a radian value. These differ from a CSS angle
string such as `"45deg"`. `%` uses JavaScript remainder semantics; `mod(a,b)` is
the separate modulo helper. Avoid zero divisors and non-finite values.

`noise1d(seed,x)` and `noise2d(seed,x,y)` take a static seed and may have dynamic
coordinates. `seededRandom(seed,count,range)` generates a prepared array; do not
call `Math.random()`. `**` is rejected; use `Math.pow`/`pow`.

| Text helper | Options |
| --- | --- |
| `formatNumber(value, { decimals, grouping }?)` | Decimals 0–12, default 0; grouping default false |
| `formatPercent(value, { decimals, grouping }?)` | Same options; `0.125` formats as `12.5%` with one decimal |
| `padNumber(value, { width })` | Zero-pad to a static width 1–64 |

Formatting options are prepare-time values. Use these helpers for frame-dependent
numeric text; locale-dependent formatting is unavailable.

## CSS and layout

### Style values and precedence

Write inline style object literals with camelCase keys; quoted kebab-case keys are
also accepted. For example `backgroundColor` and `"background-color"` name the
same property. No computed keys or spreads; write each binding explicitly.

`className` is static. Explicit styles override utility styles. Avoid conflicting
utilities and shorthand/longhand duplication; write the intended value once.
Inherited typography follows the parent where appropriate; a primitive's implied
class supplies only its documented layout shortcut.

CSS has no independent animation clock here. Drive a numeric/typed style binding
with `ctx` or a supported helper. A dynamic template literal must retain a known
CSS structure, for example `` `blur(${radius}px)` ``. For filters/gradients, a
conditional can select among finite valid structures; every branch is checked.
Do not build function names, units, selectors or complete CSS fragments from
arbitrary runtime strings.

Numeric geometry values such as `width: 120` are CSS pixels. Numbers for
`opacity`, `scale`, `flexGrow`, `fontWeight`, `zIndex` and numeric `lineHeight` have
their own unitless meanings. Use strings for keywords, percentages and compound
values. Typed lengths support `px`, `%`, `em`, `rem`, `vw`, `vh`; typed angles
support `deg`, `rad`, `turn`. Units resolve in the property context; percentages
are not universally percentages of the viewport.

For responsive geometry, use `ctx.viewport.width`/`.height` in numeric expressions.
For percentage translations use a typed pair such as `"-50% -50%"`.
When interpolating lengths, keep the units the same at corresponding stops.

The tables below describe Motion's usable authoring surface, grouped by purpose.
`S` means a static value, `D` a supported frame expression. A `D` entry does not
permit arbitrary JavaScript or CSS string construction. A parser accepting an
additional CSS property is not proof that Motion paints it correctly. The
[verification section](#examples-and-verification) distinguishes executed examples
from source-based reference coverage.

### Layout and sizing

| Properties | Values / example | Timing and behavior |
| --- | --- | --- |
| `display` | `block`, `flex`, `grid`, `inline`, `inline-block`, `inline-flex`, `inline-grid`, `none` | S/D finite keyword selection; prefer ordinary boxes for containers |
| `position` | `static`, `relative`, `absolute`, `fixed` | S/D finite keywords; `fixed` has canvas/layout semantics, no browser scrolling |
| `left`, `right`, `top`, `bottom`, `inset`, `insetInline`, `insetBlock` | `24`, `"10%"`, `"auto"`, compound insets | S/D lengths; position relative to the containing block |
| `width`, `height`, `minWidth`, `minHeight`, `maxWidth`, `maxHeight` | Pixels, percentages, `"auto"` where applicable | S/D; give media and effect surfaces explicit sizes; intrinsic keywords such as `max-content` are not supported by ordinary width/height declarations |
| `aspectRatio` | `"16 / 9"`, `"1 / 1"`, `"auto"` | S/D; use with a constrained dimension |
| `boxSizing` | `"border-box"`, `"content-box"` | S; choose explicitly when padding/border must fit a fixed size |
| `margin`, `padding`, and `Top/Right/Bottom/Left` variants | `16`, `"8px 16px"`; margin also `"auto"` | S/D; negative padding is invalid |
| `marginInline`, `marginBlock`, `paddingInline`, `paddingBlock` | One/two lengths; `InlineStart`/`InlineEnd` longhands | S/D; logical inline sides follow direction |
| `gap`, `rowGap`, `columnGap` | `16`, `"1rem"` | S/D; space between flex/grid items |
| `overflow`, `overflowX`, `overflowY` | `visible`, `hidden`, `clip`, `auto`, `scroll` | S; hidden/clip constrain painting; no interactive scrolling |
| `zIndex` | Integer | S/D; stacking/painter order, not 3D depth |
| `visibility` | `visible`, `hidden` | S/D finite keywords; `visible={...}` is also available at the JSX level |

Use `position: "relative"` on a containing box and `position: "absolute"` on
positioned children. `Scene` does not automatically center its children. In a flex
row, set `flexShrink: 0` when an authored item must keep its width. Hidden overflow
plus a border radius is the usual rounded card clip.

### Flex and Grid

| Properties | Values / example |
| --- | --- |
| `flexDirection` | `row`, `column`, `row-reverse`, `column-reverse` |
| `flexWrap` | `nowrap`, `wrap`, `wrap-reverse` |
| `flex`, `flexGrow`, `flexShrink`, `flexBasis` | `"1 1 0px"`, `1`, `0`, `"40%"` |
| `justifyContent`, `alignContent` | `start`, `end`, `center`, `flex-start`, `flex-end`, `stretch`, `space-between`, `space-around`, `space-evenly` where applicable |
| `alignItems`, `alignSelf`, `justifyItems`, `justifySelf` | `start`, `end`, `center`, `stretch`, `baseline`, `auto` where applicable |
| `order` | Integer |
| `gridTemplateColumns`, `gridTemplateRows` | `"1fr 2fr"`, `"repeat(3, minmax(0, 1fr))"` |
| `gridAutoColumns`, `gridAutoRows`, `gridAutoFlow` | Track sizes; `row`, `column`, `dense` combinations |
| `gridColumn`, `gridRow`, `gridColumnStart/End`, `gridRowStart/End` | `"1 / 3"`, `"span 2"`, line numbers or `auto` |
| `gridTemplateAreas`, `gridArea` | Named area declarations and placement |

Numeric flex fields and lengths can vary by frame. Prefer static grid structure
with animated child geometry/opacity; changing layout repeatedly can be expensive.

Save as `layout.motion.tsx`:

```tsx
const items = ["01", "02", "03", "04"];
export default function Layout(ctx) {
  return (
    <Scene className="h-full w-full" style={{ boxSizing: "border-box", padding: 32,
      backgroundColor: "#e2e8f0", display: "grid",
      gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 16 }}>
      {items.map((item) => (
        <View key={item} className="flex items-center justify-center"
          style={{ borderRadius: 16, backgroundColor: "#ffffff", border: "2px solid #cbd5e1" }}>
          <Text key={`${item}-label`} style={{ fontSize: 40, color: "#0f172a" }}>{item}</Text>
        </View>
      ))}
    </Scene>
  );
}
```

### Paint, borders and compositing

| Properties | Values / example | Timing and restrictions |
| --- | --- | --- |
| `color`, `backgroundColor` | `"#38bdf8"`, `"#38bdf880"`, CSS color literals | S/D colors; use hex for portable examples |
| `background`, `backgroundImage` | Linear/radial/conic/repeating gradients, `none`; `background` also accepts solid colors | S/D finite valid structures; no `url(...)` or unsupported interpolation spaces |
| `backgroundPosition`, `backgroundSize`, `backgroundRepeat` | `"center"`, `"100% 100%"`, `"no-repeat"` | S/D values for supported gradient layers |
| `backgroundOrigin`, `backgroundClip` | Box keywords; `backgroundClip: "text"` for text paint | S; inspect text/inline combinations |
| `backgroundBlendMode` | Supported blend-mode list for background layers | S |
| `border`, `borderTop/Right/Bottom/Left` | `"2px solid #38bdf8"` | S/D validated paint; set a style when setting widths |
| `borderWidth`, `borderStyle`, `borderColor` and side longhands | `2`, `"solid"`, `"#38bdf8"` | Width/color can animate; styles `solid`, `dashed`, `dotted`, `double`, `none`, `hidden` |
| `borderRadius`, four corner `*Radius` longhands | `16`, `"50%"`, compound/elliptical radii | S/D; clamp geometry to valid values |
| `outline`, `outlineWidth`, `outlineStyle`, `outlineColor`, `outlineOffset` | `"2px solid #38bdf8"`, `4` | S/D; inline outline islands have restrictions |
| `boxShadow` | `"0px 8px 20px #00000040"`; comma-separated shadows, optional `inset` | S/D valid CSS; can increase offscreen bounds |
| `opacity` | Number 0–1 | S/D; applies to the composed subtree |
| `mixBlendMode` | `normal`, `multiply`, `screen`, `overlay`, `darken`, `lighten`, `color-dodge`, `color-burn`, `hard-light`, `soft-light`, `difference`, `exclusion`, `hue`, `saturation`, `color`, `luminosity` | S; combines with the already-painted backdrop |
| `isolation` | `auto`, `isolate` | S; establish a compositing boundary when needed |
| `objectFit`, `objectPosition` | `contain`, `cover`, `fill`, `none`, `scale-down`; `"center"` | S for media placement |
| `imageRendering` | `auto`, `pixelated` | S; choose sampling appropriate to the image |

Avoid `groove`, `ridge`, `inset`, `outset` **border styles**; they are not emitted.
This restriction does not refer to an inset box shadow. Explicit `borderWidth`
without a non-empty border style is not a way to draw a border.
Dashed/dotted/double borders currently require uniform widths/colors around the
box; unequal per-edge paint supports solid/none/hidden styles.

CSS gradients use sRGB interpolation. Omitting the interpolation space preserves
the supported legacy behavior; explicit `in srgb` is accepted. `in oklab`,
`in srgb-linear`, Display-P3 and relative-color gradient forms are not admitted.
Multiple gradient layers use a comma-separated `backgroundImage` value.

### Transforms

| Property | Example / contract |
| --- | --- |
| `translate` | `point(x,y)`, `"12px 24px"`, `"-50% -50%"`; dynamic typed pairs accepted |
| `rotate` | `"15deg"` or angle-valued interpolation; numeric degrees via a transform template |
| `rotateX`, `rotateY` | Numeric degrees or typed angles for a projected layer |
| `scale` | `1.2` or a typed 2D scale; unitless |
| `scaleX`, `scaleY` | One axis at a time; compiler lowers to a 2D scale binding |
| `transformOrigin` | `"50% 50%"`, `point(x,y)`; defaults to center |
| `transform` | Supported translate/rotate/scale function list, including admitted 3D forms; authored order is preserved |
| `perspective`, `perspectiveOrigin` | Positive perspective distance and static/dynamic supported origin components |
| `transformStyle`, `backfaceVisibility` | Explicit `transformStyle` must be static `"preserve-3d"`; backface visibility is static `"visible"` / `"hidden"` |

For animated 2D translation, prefer `translate: point(x,y)`. A two-argument
`translate(${x}px ${y}px)` template is not interchangeable with a single typed
Length2 hole. `translateX(${x}px) translateY(${y}px)` is supported. For rotation
use `` transform: `rotate(${angle}deg)` ``; for scaling use
`` transform: `scale(${factor})` ``. Put a complete numeric argument in each hole.

Supported ordered functions include `translate`, `translateX/Y/Z`, `translate3d`,
`rotate`, `rotateX/Y/Z`, `rotate3d`, `scale`, `scaleX/Y/Z`, `scale3d`.
Use the separate `perspective` style rather than a `perspective()` transform function.
Do not assume `matrix`, `matrix3d`, `skew`, nested `calc()` inside transform
functions or arbitrary string-built transform lists work. Do not declare scale
multiple ways on the same node. To combine independent transforms without
ambiguity, put them on nested boxes.
`projectQuad` is explicitly rejected as an author API.

### Filters and animated paint example

Supported CSS filter functions are `blur`, `brightness`, `contrast`, `grayscale`,
`hue-rotate`, `invert`, `opacity`, `saturate`, `sepia`, `drop-shadow`.
They work in `filter` and `backdropFilter` through Motion's filter lowering.
Filters compose in written order. `filter` affects the node and descendants;
`backdropFilter` samples already-painted content behind the node.

Use non-negative blur lengths, finite scalar/percentage factors and explicit
angle units for hue rotation. `drop-shadow` uses offsets, optional blur and color;
it is different from a spread/inset `boxShadow`. SVG `url(...)` filters are not
supported. `none` and zero blur are valid no-ops.

Save as `paint.motion.tsx`:

```tsx
export default function Paint(ctx) {
  const t = ctx.progress;
  return (
    <Scene className="relative h-full w-full" style={{
      backgroundImage: `linear-gradient(${30 + t * 90}deg in srgb, #0f172a, #2563eb)` }}>
      <View className="flex items-center justify-center"
        style={{ position: "absolute", left: 72, top: 70, width: 496, height: 220,
        borderRadius: 28, backgroundColor: "#ffffff22", border: "1px solid #ffffff66",
        backdropFilter: "blur(12px)", boxShadow: "0px 16px 32px #00000044",
        transform: `rotate(${(t - 0.5) * 6}deg)` }}>
        <Text style={{ fontSize: 40, color: "#ffffff",
          filter: t < 0.3 ? `blur(${(0.3 - t) * 12}px)` : "none" }}>Animated paint</Text>
      </View>
    </Scene>
  );
}
```

Keep blur/backdrop regions bounded. Full-canvas blur, large shadows, deep opacity
groups and many overlapping translucent layers can dominate CPU/GPU work.

## Tailwind class catalog

Motion validates `className` against its own finite catalog. It is not a Tailwind
build pipeline: there is no config/plugin processing, arbitrary class generation,
responsive variant or interaction state. Static expressions may construct a class
string at prepare time; it cannot vary with `ctx`.

The following catalog families are admitted. `N` means a finite non-negative
number unless a narrower range is stated; it does not imply arbitrary CSS text.

| Family | Admitted forms |
| --- | --- |
| Display / position | `block`, `inline`, `inline-block`, `flex`, `inline-flex`, `grid`, `inline-grid`, `hidden`; `static`, `relative`, `absolute`, `fixed` |
| Box / visibility | `box-border`, `box-content`, `visible`, `invisible`, `isolate`, `isolation-auto` |
| Flex | `flex-row`, `flex-col`, reverse variants; `flex-wrap`, `flex-wrap-reverse`, `flex-nowrap`; `flex-auto`, `flex-initial`, `flex-none` |
| Alignment | `items-{baseline,center,end,start,stretch}`; `self-{auto,baseline,center,end,start,stretch}`; `justify-{around,between,center,end,evenly,normal,start,stretch}`; `content-{around,between,center,end,evenly,normal,start,stretch}` |
| Spacing | `gap`, `gap-x/y`; `m`, `mx/y`, `mt/r/b/l/s/e`; `p`, `px/y`, `pt/r/b/l/s/e`; `inset`, `inset-x/y`, `top/right/bottom/left`, each followed by `-N`, `-px`, `-full` |
| Automatic / negative spacing | `auto` for margins/insets; a leading `-` for margins/insets, not padding/gap |
| Sizing | `w`, `h`, `min-w/h`, `max-w/h`, `size`, `basis` followed by `-N` or `-{auto,px,full,screen,min,max,fit}` |
| Aspect | `aspect-auto`, `aspect-square`, `aspect-video` |
| Grid | `grid-cols/rows-N`, `col/row-span-N`, `col/row-start/end-N`, N 1–64; `grid-cols/rows-none`, `col/row-span-full`, `col/row-start/end-auto` |
| Grid flow / automatic tracks | `grid-flow-{row,col,dense,row-dense,col-dense}`; `auto-cols/rows-{auto,min,max,fr}` |
| Overflow | `overflow-{auto,clip,hidden,scroll,visible}` |
| Text size / alignment | `text-{xs,sm,base,lg,xl,2xl,3xl,4xl,5xl,6xl,7xl,8xl,9xl}`; `text-{left,center,right,justify,start,end}` |
| Font weight / style | `font-{thin,extralight,light,normal,medium,semibold,bold,extrabold,black}`; `italic`, `not-italic` |
| Tracking / leading | `tracking-{tighter,tight,normal,wide,wider,widest}`; `leading-N`, `leading-{none,tight,snug,normal,relaxed,loose}` |
| Text behavior | `uppercase`, `lowercase`, `capitalize`, `normal-case`; `break-{all,keep,normal}`; `whitespace-{normal,nowrap,pre,pre-line,pre-wrap}`; `truncate`, `text-clip`, `text-ellipsis`, `tabular-nums` |
| Text decoration / clamp | `underline`, `overline`, `line-through`, `no-underline`; `line-clamp-N` for 1–64 and `line-clamp-none` |
| Radius | `rounded`, `rounded-{none,xs,sm,md,lg,xl,2xl,3xl,4xl,full}`; edge/corner prefixes `rounded-t/r/b/l/tl/tr/br/bl-...` |
| Border / outline | `border`, `outline`; `border-{solid,dashed,dotted,double,none}`, same outline styles; width `border-N`, side/axis width variants, `outline-N`, `outline-offset-N` |
| Shadows | `shadow`, `shadow-{2xs,xs,sm,md,lg,xl,2xl,none}` |
| Opacity | `opacity-N`, N 0–100 |
| Gradients | `bg-linear-to-{t,tr,r,br,b,bl,l,tl}`, `bg-radial`, `bg-conic`; `from/via/to-COLOR` and `from/via/to-N%` |
| Colors | `bg`, `text`, `border`, border side/axis variants, `outline`, `decoration`, `shadow`, `text-shadow` with admitted color tokens |

Color tokens are `black`, `white`, `transparent`, `current`, or a palette family
plus shade. Families: slate, gray, zinc, neutral, stone, red, orange, amber, yellow,
lime, green, emerald, teal, cyan, sky, blue, indigo, violet, purple, fuchsia, pink,
rose. Shades: 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950. Palette colors
can use `/N` opacity, for example `bg-blue-500/20`.

Numeric spacing uses the utility scale (for example `p-4` is 16px with the default
root sizing), not literal pixel counts. Use `style` for exact geometry. Fractions
such as `w-1/2`, arbitrary brackets such as `w-[320px]`, `flex-1`, and transform/
filter utilities are not in this catalog; use `width: "50%"`, `width: 320`,
`flex: "1 1 0px"`, `transform` or `filter` styles instead.

Forbidden forms include `hover:...`, `md:...`, `dark:...`, leading/trailing `!`,
`animate-*`, `transition-*`, `transform-gpu` and `transform-cpu`. A legal utility
name still needs meaningful values and an appropriate node/layout context.
The `w-min`, `w-max` and `w-fit` utilities pass the tested render path. They are not
interchangeable with inline `width: "min-content"`, `"max-content"` or
`"fit-content"`, which the current CSS declaration parser rejects.

## Text, fonts and formulas

### Typography styles

| Properties | Values / behavior |
| --- | --- |
| `fontFamily` | Static family string or bound `asset://fontName`; cannot vary by frame |
| `fontSize`, `fontWeight`, `fontStyle` | Size in pixels/lengths; weight such as 400/700; normal/italic/oblique as accepted by the font |
| `lineHeight` | Number is a multiplier (`1.4`); length string is an explicit height (`"32px"`) |
| `letterSpacing`, `wordSpacing` | Lengths; explicit numeric pixel values are useful for spacing |
| `textAlign`, `direction` | left/right/center/justify/start/end; ltr/rtl |
| `whiteSpace` | normal/nowrap/pre/pre-wrap/pre-line; use an explicit string containing `\n` for source line breaks |
| `wordBreak`, `overflowWrap` | normal/break-all/keep-all; normal/break-word/anywhere where accepted |
| `textTransform` | none/uppercase/lowercase/capitalize |
| `textOverflow`, `lineClamp` | clip/ellipsis; positive line clamp, usually with a constrained width and overflow |
| `textDecoration`, `textDecorationLine/Style/Color/Thickness` | underline/overline/line-through, color/width; decoration style currently supports only `solid` |
| `textShadow` | CSS shadow list, for example `"0px 2px 8px #00000080"` |
| `fontFeatureSettings`, `fontVariationSettings`, `fontKerning` | Font-dependent typography; use a font containing the requested features |
| `fontVariant`, `fontVariantLigatures/Numeric/EastAsian/Caps/Position` | Static font-feature selection, for example `fontVariantNumeric: "tabular-nums"` |
| `fitText` | `fitText({ minFontSize, maxFontSize })`; see below |

Font size, colors, spacing and supported numeric styles may be animated, but text
metrics and line breaks can then change each frame. For a reveal that preserves
layout, animate opacity/translation instead of inserting/removing characters.
JSX indentation is not a reliable way to author spaces between rich runs: use
explicit `{" "}` or string expressions where spacing matters.
For a padded caption/card, put padding on a containing `View` and use flex/grid
alignment or explicit positioning. Text in an inline formatting context does not
behave like a standalone block box with vertical margins.

Native defaults include Noto text, symbols, CJK and vector color emoji fonts;
formula faces are separate. See the [font inventory](../assets/fonts/README.md).
Additional fonts must be available at measurement and rendering time. Web hosts
consume the fonts in the supplied render package; do not assume an arbitrary
operating-system font is present in every renderer.

### Rich text and per-unit animation

`Span` is a direct `Text` child. Its styles are exactly `color`, `fontFamily`,
`fontSize`, `fontWeight`, `fontStyle`, `letterSpacing`, `opacity`; it does not accept
general layout props or `className`. Multi-run text currently cannot combine with
`split/perUnit` or text-on-path. Inline `Image` is supported within Text, but
arbitrary inline container trees and transformed inline images are not.

Save as `text.motion.tsx`:

```tsx
export default function Typography(ctx) {
  const progress = interpolate(ctx.progress, [0, 0.5], [0, 1]);
  return (
    <Scene className="h-full w-full flex flex-col items-center justify-center"
      style={{ gap: 32, backgroundColor: "#0f172a" }}>
      <Text style={{ fontSize: 36, color: "#e2e8f0" }}>
        {"Growth "}<Span style={{ color: "#38bdf8", fontWeight: 700 }}>
          {formatPercent(progress * 0.245, { decimals: 1 })}
        </Span>
      </Text>
      <Text split="char" style={{ fontSize: 38, color: "#ffffff" }}
        perUnit={{ opacity: clamp(progress * 3 - ctx.unit.index * 0.12, 0, 1),
          translate: point(0, (1 - clamp(progress * 3 - ctx.unit.index * 0.12, 0, 1)) * 18) }}>
        Every letter moves
      </Text>
    </Scene>
  );
}
```

`split` is `char`, `word` or `line`; `line` refers to source newlines, not wrapped
layout lines. `split` and a non-empty `perUnit` must be supplied together.
Per-unit properties are `opacity` (0–1), `translate` (point), `scale` (point),
`rotate` (numeric angle), `color`. `ctx.unit.*` is unavailable in ordinary styles.

`Text path={path(...)}` lays out a single text run along a path. Keep it to one
line and make the path long enough; wrapped text or glyphs beyond the path are
reported as unsupported. It does not expose individual formula/glyph fragments
as arbitrary selectable JSX nodes.

### Measuring and fitting

`measureText(text, { className?, style?, maxWidth? })` runs during preparation and
returns measured metrics including `width` and `height`. Here `style` is a CSS
declaration **string**, for example `"font-size: 24px"`, not the JSX style object.
Text/options must be prepare-time values and measurements use the supplied fonts.
Do not pass frame-varying text/width or use browser measurement APIs.

`style.fitText: fitText({ minFontSize: 16, maxFontSize: 48 })` fits text to a fixed
content box with bounded font sizes. Set explicit width/height. The options are
static and require `0 < minFontSize ≤ maxFontSize`. It is a bounded layout operation,
not a loop that measures and mutates JSX repeatedly.
`fitText` owns the font size: do not declare `fontSize` on the same node.

### Math formulas

Save as `formula.motion.tsx`:

```tsx
export default function Formula(ctx) {
  return (
    <Scene className="h-full w-full flex flex-col items-center justify-center"
      style={{ gap: 24, backgroundColor: "#0f172a" }}>
      <Text style={{ fontSize: 24, color: "#94a3b8" }}>The quadratic formula</Text>
      <MathFormula latex={"x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}"}
        displayMode="display" ariaLabel="Quadratic formula"
        style={{ fontSize: 36, color: "#f8fafc" }} />
    </Scene>
  );
}
```

`latex` is static. `displayMode` is `inline` (default) or `display`; `ariaLabel`
supplies a semantic label. Use LaTeX escapes inside a JavaScript string as shown.
Fractions, roots, scripts, operators, matrices and admitted environments are
prepared into formula geometry. Animate the formula node's transform/opacity;
internal fraction/glyph parts are not individually addressable author nodes.

The math engine has an explicit admission policy: no `\includegraphics`, automatic
equation numbering, `\leqno`, or CJK/emoji inside formula text. Put ordinary/CJK text
in adjacent `Text` and images in `Image`. Chemical/proof-tree extensions depend on
the consuming build's admitted capabilities. See the
[formula fixtures](../crates/valle-compiler/tests/fixtures/motion/formulas) for
larger equations and the [capability tests](../crates/valle-compiler/tests/motion_math_formula_capabilities.rs)
for accepted and rejected extensions.

## Paths, shapes and paint

### Typed geometry

| Helper | Result / restrictions |
| --- | --- |
| `point(x,y)` | Typed 2D point; x/y may be frame expressions |
| `rect(x,y,width,height)` | Typed rectangle |
| `path(svgD)` | Typed path from static SVG path data |
| `` pathTemplate`M ${x} ${y} L ${x2} ${y2}` `` | Fixed `M/L/Q/C/Z` command topology with numeric holes |
| `line(points)` | Open polyline from typed points |
| `cubic(from,control1,control2,to)` | Cubic Bézier path |
| `arc(center,radius,startAngle,endAngle)` | Positive radius, angles in radians, sweep at most TAU |
| `area(input,baseline)` | Close a supported input path to a baseline |
| `offsetPath(input,distance)` | Geometric path offset |
| `boolean(left,right,op)` | Path union/intersection/difference/xor; both inputs must satisfy geometry admission |
| `morphPath(from,to,progress)` | Interpolate compatible path topology |
| `pointAt(path,progress)`, `tangentAt(path,progress)` | Sample path position/tangent |
| `pathLength(path)` | Path length |
| `pathTrajectory(paths,frame)` | Sample a prepared array of compatible path frames |

Use typed constructors where a Path/Point/Rect is required. Plain strings/objects
with similarly named fields are not automatically those types. For dynamic path
coordinates, use `pathTemplate` or geometry constructors instead of injecting
arbitrary commands into `d`. `deg(90)` gives a radian value for geometry APIs.

### Path and shape attributes

`Path d={...}` requires a typed path. Closed paths default to black fill. For an
open stroked path, explicitly use `fill="none"`; `Line` and `Polyline` also require
explicit fill intent.

| Attribute | Contract / default |
| --- | --- |
| `fill`, `stroke` | CSS color, typed gradient paint or `"none"`; stroke absent by default |
| `strokeWidth` | Finite non-negative width; default 1 |
| `strokeLinecap` | `butt` (default), `round`, `square`; aliases `strokeLineCap`, `strokeCap` |
| `strokeLinejoin` | `miter` (default), `round`, `bevel`; aliases `strokeLineJoin`, `strokeJoin` |
| `strokeMiterlimit` | Positive static number, default 4; alias `strokeMiterLimit` |
| `strokeDasharray` | Static number array or space-separated lengths; no negative entries or all-zero pattern; alias `strokeDash` |
| `strokeDashoffset` | Finite static/dynamic number, default 0; alias `strokeDashOffset` |
| `trimStart`, `trimEnd` | Normalized path reveal, defaults 0 and 1 |
| `arrowStart`, `arrowEnd`, `arrowSize` | Kind `none`, `triangle`, `open`; positive static size, default 8 |

Shape coordinates: Circle `cx,cy,r`; Ellipse `cx,cy,rx,ry`; Rect
`x,y,width,height`; Line `x1,y1,x2,y2`; Polyline/Polygon `points` as typed-point
arrays. Coordinates can use admitted expressions. For rounded rectangles use a
styled `View` or an authored rounded path; do not assume all SVG attributes are
available on `Rect`.

Gradient paint constructors:

- `gradientStop(offset, color)`, offset in `[0,1]`.
- `linearGradient(startPoint, endPoint, stops, spread?)`.
- `radialGradient(centerPoint, radius, stops, spread?)`.
- `conicGradient(centerPoint, startAngle, stops, spread?)`.

Use an ordered stop array with at least two stops. Spread is `pad` (default),
`repeat` or `reflect`. These typed paints belong in Path fill/stroke or Mask paint;
CSS `backgroundImage` uses CSS gradient strings.

Save as `path.motion.tsx`:

```tsx
const route = path("M 60 260 C 170 40 420 40 580 240");
export default function Route(ctx) {
  const p = interpolate(ctx.progress, [0, 0.8], [0, 1], { easing: "easeInOut" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0f172a" }}>
      <Path d={route} fill="none" stroke="#334155" strokeWidth={8} strokeLinecap="round" />
      <Path d={route} fill="none" stroke={linearGradient(point(60,260), point(580,240),
        [gradientStop(0,"#38bdf8"), gradientStop(1,"#a78bfa")])}
        strokeWidth={8} strokeLinecap="round" trimEnd={p} />
      <View style={{ position: "absolute", width: 20, height: 20, borderRadius: 10,
        backgroundColor: "#ffffff", motionPath: motionPath(route, p) }} />
    </Scene>
  );
}
```

### Follow paths and connect layout nodes

`style.motionPath: motionPath(path,progress,options?)` or `follow(...)` positions
a node along the path. Options: `anchor: "center"` (default) or `"topLeft"`,
`rotate: "auto"` (default) or `"none"`, and static numeric `angleOffset`.
Do not combine it with `transform`, `translate`, `rotate` or `transformOrigin` on the same
node. `autoRotate(path,progress)` returns the corresponding path angle when only
rotation is needed.

`bounds("key")` returns post-layout x/y/width/height. `anchor("key", side)` returns
a point; sides are left, right, top, bottom, center, top-left, top-right,
bottom-left, bottom-right. `connect(pointA,pointB,{ route }?)` builds a connection;
route is `line` (default) or `cubic`. Build orthogonal routes explicitly with `line(points)`.

Targets must be existing static node keys. Post-layout values can drive paint or
absolute overlay geometry, but cannot feed back into width/height, ordinary
layout or text content. Put a connection Path in an absolute overlay at `(0,0)`
when using scene-space anchors. See the
[architecture diagram fixture](../crates/valle-compiler/tests/fixtures/motion/charts/architecture-diagram.motion.tsx).

## Clipping, masks and effects

### Clip and Mask

`Clip path={typedPath}` clips its children using `fillRule="nonZero"` (default)
or `"evenOdd"`. The path alias `d` is accepted. It replaces CSS `clipPath` for
arbitrary path clipping; rounded rectangular clipping can use border radius and
hidden overflow.

`Mask` requires `rect={rect(x,y,width,height)}` with positive dimensions and exactly
one source:

- `paint={colorOrGradient}`;
- `src="asset://image"`;
- one direct `MaskSource` child, placed last after the content to mask.

`mode` is `alpha` (default) or `luminance`; a subtree `MaskSource` currently requires
alpha mode. A MaskSource is not independently visible content.

Save as `mask.motion.tsx`:

```tsx
const outline = path("M 50 60 L 590 60 L 590 300 L 50 300 Z");
export default function MaskedTitle(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0f172a" }}>
      <Clip path={outline} style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }}>
        <Mask rect={rect(0,0,640,360)} mode="alpha"
          style={{ width: 640, height: 360 }}
          paint={linearGradient(point(0,0), point(640,0),
            [gradientStop(0,"#ffffff00"), gradientStop(1,"#ffffffff")])}>
          <View className="h-full w-full flex items-center justify-center"
            style={{ backgroundColor: "#2563eb" }}>
            <Text style={{ fontSize: 48, color: "#ffffff" }}>Reveal with a mask</Text>
          </View>
        </Mask>
      </Clip>
    </Scene>
  );
}
```

### Displacement and motion blur

`style.displacement: displacement(seed, frequencyPoint, scale, options?)` distorts
a node and its descendants. `style.backdropDisplacement` uses the same constructor
to distort the backdrop. Seed is a static integer in `[0, 2^32−1]`; options are
`octaves` (1–4) and `mode` (`fractal` or `turbulence`). Frequency is a point,
scale a number; both may use admitted animation expressions.

`style.motionBlur: motionBlur(velocityPoint, shutterAngle?)` applies directional
blur using explicit velocity in pixels/frame. Shutter angle defaults to 180°.
This is a spatial approximation; it does not sample multiple historical scenes
or infer the velocity from a changing `left`/`rotate` property.

Effects on a group include its descendants. Keep the affected bounds small and
inspect clipping near edges. See
[advanced node effects](../crates/valle-compiler/tests/fixtures/motion/effects/advanced-node-effects.motion.tsx)
for explicit velocity and displacement examples.
The additional layer styles `paperGrain` and `contactShadow` take finite numeric
intensities clamped to 0–1. They are Motion layer effects, not browser CSS properties.

### Layout transitions

Save as `flip.motion.tsx`:

```tsx
const layouts = defineLayoutStates({
  small: { card: rect(40,100,180,160) },
  large: { card: rect(280,60,320,240) },
});
export default function Flip(ctx) {
  const p = interpolate(ctx.progress, [0,0.7], [0,1], { easing: "easeInOut" });
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0f172a" }}>
      <View className="flex items-center justify-center" layoutId="card"
        style={{ layoutTransition: flip(layouts,"small","large",p),
        backgroundColor: "#2563eb", borderRadius: 24 }}>
        <Text style={{ fontSize: 28, color: "#ffffff" }}>One card</Text>
      </View>
    </Scene>
  );
}
```

`defineLayoutStates` declares 2–16 static named states, each containing the same
1–512 layout IDs with positive Rect values. A `View layoutId="..."` must consume
that ID through `style.layoutTransition: flip(layouts,from,to,progress)`.
The transition owns the node geometry; do not also author its position/size/
translation/scale fields. State names are static; progress may animate.

### Glass

`Glass` describes an optical surface with a stable `surfaceId` and explicit shape.
`GlassField` groups surfaces under a shared `fieldId` and material. This is a
separate effect from CSS `backdropFilter: "blur(...)"`.

| Attribute | Contract |
| --- | --- |
| `shape` | `{ kind: "circle" }`, `{ kind: "capsule" }`, `{ kind: "continuousRect", radius }`, or admitted closed polygon path shape |
| `material` | `clarity`, `depth`, `tint`; normalized material controls and a color |
| `motion` | `character: responsive/fluid/viscous/elastic`, `intensity`, `settle` in seconds, optional `drive` |
| `motion.drive` | `translation: point(...)`, `pressure`, `twist` |
| `presence` | Number 0–1, default 1 |
| `environment` | `light: { direction: point(...), elevation, intensity, space: "world" or "screen" }` |
| `foreground` | `tone: auto/light/dark/none`, normalized `protection` |
| `GlassField merge` | `{ distance: number }`, static, default 24; members retain separate surface IDs |

Save as `glass.motion.tsx`:

```tsx
export default function GlassCard(ctx) {
  return (
    <Scene className="relative h-full w-full"
      style={{ backgroundImage: "linear-gradient(30deg, #0f172a, #2563eb, #38bdf8)" }}>
      <Glass className="flex items-center justify-center"
        surfaceId="card" shape={{ kind: "continuousRect", radius: 28 }}
        material={{ clarity: 0.8, depth: 0.4, tint: "#b9d7ff18" }}
        style={{ position: "absolute", left: 100, top: 80, width: 440, height: 200 }}>
        <Text style={{ fontSize: 40, color: "#ffffff" }}>Glass material</Text>
      </Glass>
    </Scene>
  );
}
```

For bindings that affect the material's motion history, use continuous
`ctx.seconds`/`ctx.progress`; discrete `localFrame` and phase clocks are restricted
there. The engine owns causal material sampling. Avoid using a frame counter as a
replacement for this time contract. Surface IDs remain stable across frames.

`motion check` prepares the Glass material through the same temporal preparation
path as native rendering. Check representative frames as well as the opening frame.

## Data visualization and repeated geometry

### Prepared compute helpers

These helpers compute data/geometry during prepare. They are available globally;
they do not import a browser charting library or execute a simulation each frame.
Animate the resulting ordinary View/Text/Path nodes afterward.

| Helper | Inputs and result |
| --- | --- |
| `extent(values)` | Numeric min/max; empty data needs an explicit policy |
| `extentOrDefault(min,max)` | Stabilize a degenerate numeric domain |
| `ticks(start,stop,count)`, `niceDomain(start,stop,count)` | Tick values / expanded numeric domain |
| `scaleLinear({ domain:[a,b], range:[x,y] })` | `.map(value)`, `.invert(value)`, `.ticks(count)` |
| `scaleBand({ range:[a,b], count, paddingInner?, paddingOuter?, align? })` | `.band(index)` pair, `.center(index)`, `.step()`, `.bandwidth()` |
| `scalePoint({ range:[a,b], count })` | `.at(index)`, `.step()` |
| `stack(values,{ offset }?)` | Stacked intervals; default offset `zero` |
| `geoProject({ points, projection?, width, height, padding? })` | Batch projection/fitting of geographic coordinates |
| `geoPath({ polygons, projection?, width, height, padding?, clip? })` | Projected polygon paths |
| `placeLabels(candidates,{ bounds, padding? })` | Deterministic placement of label candidates |
| `graphLayout({ sizes, edges, nodeGap?, rankGap? })` | Graph ranks, rows, back edges and node centers |
| `seededRandom(seed,count,range)` | Prepared pseudorandom values with explicit seed |

Scale and layout calls take prepare-time inputs. To animate a scale-mapped value,
map the data once and multiply by a frame-dependent reveal, as in the bars example.
Project an entire geographic batch together so all features share a fit. Defaults:
geo projection `albers`, padding `0.04`; label padding `4`; graph node gap `40`,
rank gap `64`.

`stack` takes a rectangular matrix `values[series][category]` and returns
`layers[series][category] = [low, high]`, plus `positiveTotals` and `negativeTotals`.
Offset `zero` supports positive/negative stacks; `expand` normalizes non-negative
categories to 0–1. `placeLabels` takes candidates shaped as
`{ point: [x,y], size: [width,height], priority }`, bounds `[left,top,right,bottom]`,
and returns `{ x, y, visible }` per candidate in input order. Measure text before
placing its label; hidden candidates still need stable scene keys.

Detailed data shapes and executable compositions are in the
[chart fixtures](../crates/valle-compiler/tests/fixtures/motion/charts),
[map fixture](../crates/valle-compiler/tests/fixtures/motion/maps/map-atlas.motion.tsx)
and [data dashboard](../crates/valle-compiler/tests/fixtures/motion/modules/data-dashboard/dashboard.motion.tsx).
Valle exposes these lower-level builders; it does not currently provide a general
`Chart`, `FunctionPlot` or arbitrary HTML widget primitive.

### GeometryBatch and particles

Use `GeometryBatch` when many circles/rectangles share one coordinate space and
do not need separate layout/children. Ordinary `.map()` nodes remain useful for
individually styled cards, text and other content.

`geometry` is `circle` or `rect`. `positions` is a prepared point array or a
supported field. `sizes` and `fills` are required; scalar values broadcast across
instances, while arrays follow the batch contract. `opacities` is optional;
`semanticKeys` provides static per-instance identities. Do not give it JSX children.

Save as `particles.motion.tsx`:

```tsx
export default function Particles(ctx) {
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0f172a" }}>
      <GeometryBatch geometry="circle"
        positions={particles(ctx.localFrame, ctx.fps, {
          seed: 17, count: 240, emitter: rect(300,300,40,4),
          birth: { interval: 0.01 }, lifetime: 2,
          velocity: { x: [-80,80], y: [-180,-80] },
          gravity: point(0,90), loop: true,
        })}
        sizes={[2,6]} fills={["#38bdf8","#a78bfa"]} opacities={[1,0]}
        style={{ position: "absolute", left: 0, top: 0, width: 640, height: 360 }} />
    </Scene>
  );
}
```

`particles(frame,ctx.fps,options)` is admitted as a batch position field. Options
are prepare-time values: integer seed/count, typed emitter Rect, positive
`birth.interval` and `lifetime` in seconds, velocity ranges in pixels/second,
typed gravity and optional loop. With particle fields, two-element sizes, fills
and opacities describe the start/end values over particle life. It is deterministic
at an arbitrary frame; it does not require rendering every previous frame.
For looping fields, `count × birth.interval` must be at least `lifetime`.

For fixed batches, `field({ from, to, progress, stagger? })` can bind positions,
sizes, fills or opacities. From/to are static compatible arrays (or admitted
broadcast values for side fields); progress is numeric and can animate. Stagger
is a static non-negative delay, and the final instance's delay must remain below
1. Keep array lengths and geometry types consistent. See
[batch attribute lowering](../crates/valle-compiler/src/motion/attrs.rs) for type
admission when constructing a new field.

## Camera, 3D and shaders

### 2D scene camera

Save as `camera.motion.tsx`:

```tsx
export default function Camera(ctx) {
  const zoom = 1 + ctx.progress * 0.4;
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#0f172a" }}
      camera={{ center: point(320,180), zoom, rotation: 0 }}>
      <World>
        <View style={{ position: "absolute", left: 230, top: 90,
          width: 180, height: 180, borderRadius: 24, backgroundColor: "#2563eb" }} />
      </World>
      <Screen>
        <Text style={{ position: "absolute", left: 24, top: 24,
          fontSize: 24, color: "#ffffff" }}>Fixed screen label</Text>
      </Screen>
    </Scene>
  );
}
```

`camera={{ center, zoom, rotation }}` moves the world at the Scene boundary.
Center is a point, zoom stays positive, rotation is numeric degrees. Screen-space content
stays fixed. `Screen` must be a direct Scene child after all world content; do not
nest World inside Screen. Camera changes affect painting, not the world's layout
boxes. Use camera movement for a viewport pan/zoom rather than changing every node.

### Scene3D

`Scene3D` rasterizes a bounded 3D scene into a composited leaf. Give it explicit
width/height and `camera={{ position:[x,y,z], target:[x,y,z], ... }}`.
Position/target components, `orbitYaw`, `orbitPitch`, `distance`, `fov`, `near`
and `far` may use frame expressions. Motion resolves orbit controls into the final
position before submitting the frame; the renderer receives one complete camera.
FOV defaults to 38° and is restricted to 10–120°; near defaults to 0.1.

Its direct children are explicit leaves, not arbitrary React/Three.js content:

| Child | Attributes |
| --- | --- |
| `Mesh` | Static `key`, `src="asset://model"`; `material` and indexed `materials` overrides; animated `position`, `rotation`, `scale` triples and `translateX/Y/Z`, `rotateX/Y/Z`, `scaleX/Y/Z`; model-node `nodes` bindings |
| `Anchor3D` | Static `key`, parent mesh key and position triple |
| `AmbientLight` | Optional key, opaque `color` (default white), `intensity`; color and intensity may animate |
| `DirectionalLight` | Animated nonzero `direction` triple, optional key, color (default white), intensity |
| `HemisphereLight` | Opaque `skyColor` / `groundColor`, optional `direction` (default +Y), key/intensity; colors, direction and intensity may animate |

Models use a declared `model3d` asset and the admitted GLB format. With no material
attributes, a Mesh uses the model's metallic/roughness materials. `material` supplies
an override for the whole Mesh; `materials={[{id: 0, ...}]}` supplies overrides for
original GLB material indices. Merge order is source material, whole-Mesh override,
then indexed override. Missing properties inherit; supplied values replace them.
Primitives without a source material index receive the whole-Mesh override.

```tsx
<Mesh key="model" src="asset://model"
  material={{ type: "pbr", roughness: 0.3 + ctx.seconds * 0.1,
    textures: { baseColor: "asset://paint", normal: "asset://normal" } }}
  materials={[{ id: 0, color: "#2878ff", metallic: 0.8,
    emissive: "#102040", emissiveIntensity: ctx.seconds * 0.2,
    alphaMode: "mask", alphaCutoff: 0.5, doubleSided: true,
    textures: { normal: null } }]} />
```

`type` is `unlit`, `lambert` or `pbr`. Colors and numeric material properties may use
frame expressions: `color`, `metallic` (0–1), `roughness` (0–1), opaque `emissive`,
`emissiveIntensity` (0–16), `normalScale` (−16–16), `occlusionStrength` (0–1) and
`alphaCutoff` (0–1). CSS colors are interpreted as sRGB and converted to linear RGB.
Every frame carries the complete set of authored overrides, evaluated before drawing;
source materials and prepared images remain immutable.

Texture controls have kind `image` and accept PNG/JPEG. The `textures` map has five
slots: `baseColor` (sRGB RGB + linear alpha), `metallicRoughness` (G roughness/B
metallic), `normal` (tangent-space RGB), `occlusion` (R), and `emissive` (sRGB RGB).
All slots use `TEXCOORD_0`. Omit a slot to inherit it, or set it to `null` to remove it.
A texture can be an asset URI or `{ src, wrapU, wrapV, minFilter, magFilter, mipmap }`.
Wrap modes are `clamp`/`repeat`/`mirror`; min/mag filters are `nearest`/`linear`; mipmap
selection is `none`/`nearest`/`linear`. Defaults are repeat wrap and linear filtering.
Current-frame UV gradients select the mip level, including each input's own size.

Embedded and external images use the same Rust decoder and sampler. Color and data
roles have separate cache identities and mip chains. Float RGBA storage preserves
16-bit PNG values and RGB under transparent alpha; color conversion happens before
linear mip filtering. Storage budgets include every mip at 16 bytes per pixel.
Host image fulfillment verifies encoded content digests before registration. Portable
hosts must retain the referenced image bytes alongside the fixed semantic package.

`alphaMode` and `doubleSided` are static material configuration. All material kinds
support opaque and mask modes. Opaque ignores base alpha. Mask multiplies factor alpha
by texture alpha, discards values below the cutoff (default 0.5), and writes no
color/depth/picking data for discarded fragments. Retained fragments are opaque.
Double-sided materials render both faces with reversed back-face normals. Use the
Scene3D layer's 2D opacity for compositing transparency; sorted 3D blending is outside
this profile. Normal and occlusion maps share the same semantics for lit materials;
occlusion affects indirect illumination.

Mesh vector attributes may contain frame expressions. Axis translations and rotations
add to the corresponding vector components; axis scales multiply them. The compiler
combines these into one complete transform. Rotation uses degrees and `T * Rz * Ry * Rx * S`.
Scale may be negative; its magnitude must remain between 0.000001 and 1000, with no
zero crossing. No mutable pose state is retained between frames.

To animate model nodes, bind their original GLB indices:

```tsx
<Mesh key="model" src="asset://model" material={{ type: "pbr", color: "#ffffff" }}
  nodes={[
    { id: 2, position: [0, ctx.seconds * 0.1, 0], rotation: [0, ctx.seconds * 30, 0], scale: [1, 1, 1] },
  ]} />
```

Each entry **replaces the node's complete local transform**: omitted position/rotation
are zero and omitted scale is one. Unbound nodes retain their frozen source transforms.
The hierarchy is recalculated from these inputs on every frame; parent transforms affect
all descendants, while instances referencing the same mesh can move independently.
IDs must be distinct and reference active nodes in the selected model scene. No external
animation player or imported glTF animation track is involved.

Static GLB scenes admit multiple roots, parent/child nodes and shared mesh instances.
Node TRS and column-major affine matrices remain separate from source geometry;
matrices must decompose into non-singular TRS. Negative scales are supported, with
inverse-transpose normals and mirrored winding. Node identifiers are their original
glTF indices, independent of display names and traversal order. The importer retains
up to 256 nodes/meshes and a maximum hierarchy depth of 32. Picking returns the
scene object's semantic address plus `nodeId`, the original glTF node index.
The raster keeps separate object/node planes, with ten bytes per pixel for color,
depth and both identifiers; buffer reuse resets every plane.

Triangle primitives accept packed or interleaved FLOAT positions/normals, FLOAT or
normalized U8/U16 UVs, U8/U16/U32 indices and non-indexed geometry. Missing normals
produce flat face normals with split corners. UVs may be absent when no material
texture needs them; binding a texture without UVs reports an error. Buffer-view targets
are optional, and present targets must match their use. External URIs, sparse accessors,
skins, morphs, extensions, imported animation and alpha-blended GLB materials remain
outside the admitted profile.

Environment lighting uses an explicitly bound `asset({kind: "environment"})` resource:

```tsx
<Scene3D pbr={{
  environment: { src: "asset://sky", intensity: 1, rotation: ctx.seconds * 30, background: false },
  toneMapping: "aces", exposure: 1.1,
}} /* camera and model children as above */ />
```

Bind an equirectangular HDR, PNG or JPEG panorama with `--asset sky=sky.hdr`.
The source must be 2:1 and no larger than 4096×2048. HDR RGB is linear sRGB;
PNG/JPEG RGB is interpreted as sRGB. Alpha is ignored for lighting. RGB must be
finite within 0–65504. Admission deterministically produces diffuse cube faces
and reflection mips and freezes the bytes with source identity and preprocessing
settings. Runtime reads these immutable resources without reopening the source.

Intensity (0–16) and Y rotation in degrees may use frame expressions. Background
visibility is independent of illumination. Tone mapping (`none` or `aces`) and
exposure (0–16, default 1) are applied once in linear light before scene output.
Tone mapping is static; exposure may animate. Ambient, directional and hemisphere
light colors are converted from sRGB to linear values for both Lambert and PBR.


The loader retains a 16 MiB GLB / 65,535 vertex / 20,000 triangle limit. A layer
admits at most 8 textures, 24 Mi pixels and 128 MiB decoded mip/environment storage.
Geometry storage is shared between nodes, while vertex and triangle work is charged
for every rendered instance. Over-budget inputs fail before drawing.

`project3d("sceneKey","meshKey::anchorKey")` returns a post-layout 2D point for an
overlay. The same paint-only dependency rules as `bounds` apply. See
[Scene3D admission](../crates/valle-compiler/src/motion/scene3d.rs) and
[model/raster tests](../crates/valle-motion/tests/scene3d_model_raster.rs) for the
asset and renderer contract.

### ShaderLayer

Bind a `.shader.json` descriptor through an `asset({ kind: "shader" })` control.
Its `entry` is a relative `.vsksl` source path. The compiler validates and freezes
both files, computes their content and ABI hashes, and includes them in the render
package. Author files declare inputs, uniform types/defaults/ranges, and the
output contract; they carry no versions or hand-maintained hashes.

```tsx
export const controls = defineControls({
  assets: {
    effect: asset({ kind: "shader", required: true }),
    noise: asset({ kind: "image", required: true }),
  },
});
export default function Dissolve(ctx) {
  return (
    <Scene>
      <ShaderLayer source="asset://effect" inputs={{ noise: "asset://noise" }}
        uniforms={{ progress: ctx.progress, edgeWidth: 0.06, edgeColor: "#38bdf8" }}
        style={{ width: 520, height: 200 }}>
        <View style={{ width: 520, height: 200, backgroundColor: "#ffffff" }} />
      </ShaderLayer>
    </Scene>
  );
}
```

For the [local-dissolve descriptor](../crates/valle-compiler/tests/fixtures/motion/shader-packages/local-dissolve/manifest.json),
copy the descriptor as `effect.shader.json` and keep its source beside it. Render
with `--asset effect=effect.shader.json --asset noise=noise.png`. Timeline uses the
same asset controls through the clip's `resources` mapping.

Shader source defines `float4 valle_main(float2 uv)` (`half4` is accepted as a
float alias). UV is local to the layer and normalized; `resolution` gives the
layer's pixel size. The typed language supports local variables and constants,
blocks, `if/else`, helpers in any definition order, scalar/vector arithmetic,
boolean and integer values, and square 2×2/3×3/4×4 matrices. Functions return a
value on every path. Texture access uses `sampleContent(uv)` and declared
`sample_<input>(uv)` helpers. Each declared input has its own dimensions,
`sampling: "nearest" | "linear"`, and `wrap: "clamp" | "repeat" | "mirror"`
(default clamp). Linear filtering interpolates premultiplied linear texels before
returning a straight color; it counts four texel reads toward the sampling budget.

Set an input's `kind` to `"color"` (the default) or `"data"`. Color inputs follow
these color/alpha rules. Data inputs decode PNG/JPEG channels as normalized numbers,
including 16-bit PNG values and RGB under zero alpha. They ignore color profiles,
transfer functions and orientation metadata; linear filtering interpolates each
channel independently. The same image can be bound once as color and once as data;
those interpretations have separate resource identities and decode caches.

Data inputs use a shared byte-plane representation to preserve numeric values
through both backends' shader interfaces. Nearest sampling costs eight texel reads;
linear sampling costs 32. Their storage is eight bytes per source pixel, limited
to 64 MiB and 4096 pixels per source edge. Storage is reserved together with all
intermediate surfaces against the frame memory limit, including on cached frames.

Content coordinates follow the layer through translation, rotation and scaling.
The complete subtree is available for sampling, including pixels outside the final
viewport. Sampling outside the content returns transparent pixels. A layer can
also omit children and generate pixels entirely from its shader.

For output beyond the border box, set `output.padding: [left, top, right, bottom]`
in the descriptor (non-negative integer local pixels; default `[0, 0, 0, 0]`).
Padding expands the output rectangle without changing `resolution` or the UV
origin. Both padded area and full input surfaces count toward allocation budgets;
the final viewport and explicit ancestor clips still clip the output. Each local
shader output, including padding, is limited to 2,073,600 pixels.

`for (int i = start; i < end; i += step)` also supports `<=`, `>`, `>=`, `++`,
`--`, and `-=`. Bounds and step are compile-time integers. Nested loops are
allowed; their counters are read-only inside the body. Recursion, dynamic loops,
mutable globals, external I/O, and dynamic indexing are rejected with a source
location. Vector/matrix indices must be constant and in range.

Uniforms support `float`, `float2`, `float3`, `float4`, `float2x2`, `float3x3`,
`float4x4`, `color`, and `bool`. Matrices are flat column-major arrays; each
numeric component can be a frame expression. Numeric ranges and finite values
are checked after frame evaluation, before drawing. Time and randomness come
from explicit frame uniforms and seeds.

Color calculations use **linear sRGB with straight alpha**. The descriptor's
output is `{ colorSpace: "linear-srgb", alphaMode: "straight", allowTransparent: true }`.
`color` uniforms decode author colors; vector uniforms preserve numbers.
`srgbToLinear(float3)` and `linearToSrgb(float3)` provide explicit transfer
functions. The output boundary converts once to premultiplied linear Rec.2020;
RGB is not clipped to the display range. Zero division and zero normalization
return zero. Invalid square-root/log inputs and negative-base powers return
zero; non-finite output channels become zero and alpha is clamped to [0, 1].

Host limits are centralized in `shader/dialect.rs`: 32 KiB source, 32 functions,
16 nested calls, 256 iterations per loop, 4096 scalar operations, 64 texture
samples, and 256 expanded calls per pixel. Branches use the maximum path cost;
loops and helper calls include their expanded cost. See the
[contract tests](../crates/valle-motion/tests/shader_package_contract.rs) for
accepted programs and rejection cases.

## Troubleshooting and limits

| Symptom / diagnostic | What to change |
| --- | --- |
| `ModuleShape` | Use a function default export and supported parameter/export names |
| `GrammarForbidden` on a list | Prepare its input first, use a synchronous map and add stable keys |
| `GrammarForbidden` on a style | Use an explicit style object, supported types and closed CSS structure |
| `TailwindForbidden` / `TailwindUnsupported` | Consult the catalog; replace the utility with an explicit supported style |
| `SandboxForbidden` | Move I/O/time/locale-dependent work outside Motion; bind its results as data |
| `StaticEvalFailed` / `BuiltinRejected` | Check argument shape, prepare-time requirements, ranges and finite values |
| Missing image/font/video | Declare the matching asset control and pass `--asset name=path` |
| Text looks different after export | Match font resources, canvas, duration and FPS; inspect wrapping/fallback |
| Border is missing | Set `borderStyle: "solid"` or a complete border shorthand |
| Open path fills an unexpected shape | Set `fill="none"` for stroke-only intent |
| A blur is rejected at certain frames | Clamp its computed radius to non-negative values |
| Post-layout dependency error | Keep bounds/anchors out of size, ordinary layout and text content |
| PNG and MP4 show different animation states | Match all render inputs and compare the same zero-based frame |
| Slow export | Inspect bounded layers/blur/overdraw first; select backend/workers and encoding separately |

Unsupported authoring patterns include React hooks/effects, DOM access, browser
events, external CSS stylesheets/selectors/media queries, CSS keyframe animations,
arbitrary npm modules, runtime network/filesystem access, unseeded randomness,
dynamic resource identities and frame-varying tree sizes.

Use `Clip`/`Mask` instead of CSS `clip-path`/`mask-image`; use `motionPath` instead
of CSS `offset-path`; use supported transforms rather than matrix/skew strings.
An upstream CSS parser may recognize an otherwise unsupported declaration. Always
check the rendered frame and any prepare/render diagnostics; `motion check` cannot
prove every future frame's numeric validity or every backend's visual behavior.

Current compiler budgets include 10,000 items per static JSX map, 50,000 expanded
list items across compilation, 2,000 helper expansions, 4,096 items for expression
map unrolling and expression nesting depth 256. Themes have bounded depth/size;
sequences allow at most 256 stages. These are rejection ceilings, not performance
targets. Prefer smaller layouts or explicit batches for large repeated geometry.

For export controls, `--backend raster` selects CPU composition, `metal` requires
Metal and `auto` chooses an available backend. `--workers` accepts 1–8; defaults
are at most two for Raster and one for Metal. `--hardware-encode` is independent
of the compositor and fails if the requested hardware encoder is unavailable.
`--ffmpeg-log-level info` adds encoder diagnostics on stderr; `--events` adds
machine-readable progress on stdout. See [CLI output](cli.md#output-contract-for-scripts-and-agents).

## Examples and verification

Complete snippets above are intended to be saved as the named `.motion.tsx` files.
Unless another command is shown, inspect them at 640×360, duration three seconds,
30 FPS, frame 45, using a fresh PNG output path. Try early/late frames for animated
scenes. `poster` needs an image, `bars` its JSON, and `Dissolve` its package/image.
Source-only snippets or constructor signatures are not standalone scenes.

This reference follows the compiler, layout adapter and native renderer. The
[authoring regression tests](../crates/valle-compiler/tests/motion_authoring_regressions.rs)
cover list identity and unknown style fields. The
[CLI regression tests](../crates/valle-cli/tests/timeline_motion_regressions.rs)
exercise Glass, ShaderLayer, explicit cues and per-instance image bindings.
Use actual previews for property combinations and frames beyond these cases.

Existing examples provide larger compositions without duplicating them here:

| Task | Reference |
| --- | --- |
| Minimal title | [hello.motion.tsx](../examples/hello.motion.tsx) |
| Named multi-stage animation | [sequence product tour](../crates/valle-compiler/tests/fixtures/motion/authoring/sequence-product-tour.motion.tsx) |
| Font-bound title and formatted values | [rich metric title](../crates/valle-compiler/tests/fixtures/motion/authoring/rich-metric-title.motion.tsx) |
| Procedural values/noise | [procedural dashboard](../crates/valle-compiler/tests/fixtures/motion/authoring/procedural-dashboard.motion.tsx) |
| Layout transitions | [dashboard FLIP](../crates/valle-compiler/tests/fixtures/motion/animation/dashboard-flip.motion.tsx) |
| Repeaters/trails | [modifier logo echo](../crates/valle-compiler/tests/fixtures/motion/animation/modifier-logo-echo.motion.tsx) |
| Animated geometry | [path formation](../crates/valle-compiler/tests/fixtures/motion/animation/path-formation.motion.tsx) |
| Data, modules and themes | [data dashboard](../crates/valle-compiler/tests/fixtures/motion/modules/data-dashboard/dashboard.motion.tsx) |

For maintainers, changes to the public surface should update this guide together
with a meaningful example/test. Source owners:
[JSX primitives](../crates/valle-compiler/src/motion/jsx.rs),
[controls](../crates/valle-compiler/src/motion/controls.rs),
[expressions](../crates/valle-compiler/src/motion/expr.rs),
[styles](../crates/valle-compiler/src/motion/style.rs),
[Tailwind admission](../crates/valle-motion/src/tailwind.rs),
[layout admission](../crates/valle-motion/src/layout/scene.rs),
[drawing emission](../crates/valle-motion/src/emit.rs).
