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
export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };

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
valle motion check title.motion.tsx --json
valle motion render title.motion.tsx --frame 30 -o title.png --backend raster --json
valle motion studio title.motion.tsx
valle motion render title.motion.tsx -o title.mp4 --events
valle motion render title.motion.tsx -o title.mov
valle motion render title.motion.tsx -o title-prores.mov --codec prores4444
valle motion render title.motion.tsx -o 'frames/%05d.png'
valle motion render title.motion.tsx --frames 0,30,59 --storyboard sheet.png --json
```

`motion check --frame N --json` reports `expressions`, `fullEvaluationExpressions`,
`evaluatedExpressions`, `activeNodes`, and `layoutNodes` for the checked source sample.
`expressions` counts the compiled expression pool, including each instance template once;
`fullEvaluationExpressions` counts the work if every instance row were evaluated.
`evaluatedExpressions` counts the selected frame work, including visibility gates and
pre-layout expressions. Post-layout and per-unit passes are outside this diagnostic.
Independent hidden absolute leaves can be skipped; one instance batch counts as one
scene node even when it contains many visible rows.
The load-time scene dependency graph records geometry reads,
mask paint sources, composition scopes, background reads, and sample dependence used by
this decision. See [time functions and visibility evaluation](#time-functions-and-visibility-evaluation)
for recognized time-dependent gates.

`--frame` selects one zero-based frame as PNG. Full exports select the codec from
the extension: `.mp4` uses H.264; `.mov` defaults to lossless qtrle with straight RGBA.
Use `--codec prores4444` for a transparent ProRes 4444 `.mov`. PNG and MOV preserve the
scene's alpha; MP4 composites the scene over black. Transparent MOV delivers video
only; Timeline audio must be exported separately or delivered with MP4.

A PNG pattern containing exactly one `%d` or `%0Nd` field in its filename writes
all frames (`N` is 1–20). Numbering uses the original zero-based frame key, so
`--frames 0,30,59 -o 'frames/%05d.png'` writes `00000.png`, `00030.png`, and
`00059.png`. Parent directories are created automatically. `--frames` preserves
the requested order; duplicate keys are allowed in a sheet but rejected for a
sequence because they name the same output file.

`--storyboard sheet.png` uses the requested frames, or up to 12 evenly spaced
frames including the first and last when `--frames` is omitted. Cells use the
full delivery dimensions (or `--output-size`), with up to four columns; unused
cells are transparent. `-o` is optional for a sheet. Supplying both a PNG pattern
and a sheet writes both from the same rendered pixels. A sheet is limited to
512 MiB of RGBA pixels; reduce `--output-size` or select fewer frames if needed.

The source is compiled once for the complete delivery. JSON reports expose the
actual compiler-entry count as `compilations`, exact image `delivery.frameKeys`,
`delivery.outputs`, and compile/prepare/render milliseconds under `delivery.timing`.
`--events` emits one `motion.compilation` event per actual compile.

Destinations must be new files. PNG sequences and sheets are staged before
publication; a failed render leaves no completed outputs. Pure graphics PNG
rendering needs no FFmpeg; video encoding and decoding need compatible FFmpeg shared libraries. See
[runtime setup](cli.md#runtime-notes).

Canvas and duration come from the file's `composition`; `fps` is optional there and
can be supplied or overridden with `--fps`. A standalone file without a composition
is an error. `--output-size` scales delivery without changing layout or
`ctx.viewport`. `duration` is in seconds; output frames are the nearest frame
boundary at the actual FPS. For example, 4.6 seconds at 24 FPS produces 110 frames.
Keep data, assets and fonts consistent between previews and export; a rational
`fps` such as `30000/1001` is accepted in `composition` and `--fps`.

`motion studio` opens the same Timeline editor as `timeline studio` and
`project studio`. A standalone file starts with one selected Motion visual clip.
The component panel writes supported `composition` values and declared prop
defaults back to JSX; the code pane edits the source directly. Preview compiles
the current source in a browser Worker. The ordinary **Save** action writes source
files; arrangement changes can be stored with **Save as Timeline**. The resulting
JSON contains the explicit `--props`, inline `--data`, `--asset` bindings and
effective frame rate, with resource paths resolved relative to the saved JSON.
Extra `--font` stacks cannot be represented by a Timeline clip, so Studio disables
that conversion until those fonts are declared and bound as font assets. Source
editing and source saving remain available.

## Language and components

### Module and function shape

The entry module needs a named default function declaration and a
`composition` with its canvas and duration. Optional `export const controls =
{ ... }` declares external inputs. Helper modules may export reusable values
and components.

The root parameter positions are `(ctx, props, data)`. Omit unused trailing
parameters. `props` and `data` may be destructured in their positions. An authored
child component uses `(ctx, props)`; pass prepared data through its props.
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
| Spread | Fixed-shape style objects support spreads. JSX prop/per-unit spread and list/interpolation array spreads or holes are not admitted |

Prepare-time means independent of `ctx` and runtime `props`. Module
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
| `Scene` | Composition root; optional 2D `camera` and static `bloom` |
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
| `GeometryBatch` | Repeated circles/rectangles/paths; `geometry`, `positions`, `sizes`, `fills`, optional `opacities`, `rotations`, `skewXs`, `strokeWidths`, `semanticKeys` |
| `Clip` | Path-clipped children; `path` (or `d`), `fillRule` |
| `Mask`, `MaskSource` | Alpha/luminance masking; see [masks](#clipping-masks-and-effects) |
| `MathFormula` | Formula leaf; static `latex`, `displayMode`, `ariaLabel` |
| `Glass`, `GlassField` | Optical material surfaces and shared material fields |
| `Transition` | Two child subtrees, static `kind`, numeric `progress`, optional `params`; see [transitions](#subtree-transitions) |
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
| `data` | Validated structured JSON available during prepare through the third root argument |
| `assets` | `asset({ kind, required })`, referenced by `asset://name` |

Prop constructors: `number`, `string`, `boolean`, `color`, `length`, `angle`,
`point`, `rect`, `select`. Common options are `default`,
`required`, `label`; numbers add `min`, `max`, `step`; `select` requires a string
`values` array. Examples: `number({ default: 24, min: 0 })`,
`select({ default: "light", values: ["light", "dark"] })`,
`point({ default: point(40, 60) })`.

The constructors `point`, `rect` and `boolean` have schema forms
distinct from their geometry forms. `path(svgD)` draws path geometry, and
`frames(n)` supplies a sequence time value.
Length and angle defaults are strings such as `"24px"` and `"15deg"`.

Use `--props props.json` for constant prop values and `--data data.json` for
structured preparation input, alongside `--asset` and `--font`.
Required inputs without defaults must be supplied. See
[Timeline integration](timeline.md#motion-integration) for animated prop curves
and clip placement. `motion check` uses the same preparation
and Native Raster rendering path for one frame (default `--frame 0`); pass the same
data, asset and font bindings as the intended render, and keep them consistent with the
file's `composition`. It creates no persistent output and does not prove that every frame
or encoder will succeed.

`valle motion review scene.motion.tsx --json` checks the full output duration for
screen-position strobing after layout and transforms. It reports a `strobe` issue when a
visible painted node moves over a quarter of its extent along the travel direction per frame for at least three consecutive frame
intervals without motion blur. It also reports `text_readability` when visible text is
shown for less than 0.3 seconds plus 0.3 seconds per word or 0.225 seconds per CJK character, `text_out_of_frame` for text
outside the canvas, and `text_overlap` for overlapping glyph ink bounds. Informational
`motion_while_reading` and `simultaneous_main_actions` hints identify competing prominent
motion over consecutive frames. `linear_motion` flags a prominent subject whose long move has
nearly constant screen speed. `stagger_timing` compares observed starts in a consecutively
numbered moving group against a 30–100 ms gap. The same bindings and FPS options apply. `--trajectories`
adds screen positions, bounds, velocity and acceleration
per visible frame to the JSON report. `--trajectory-sheet review.png` renders a contact sheet
with the same trajectories and sampled frame markers; see the
[CLI guide](cli.md#motion-jsx-to-video) for the frame budget and report details.

Audio assets can drive Motion through `audioAnalysis("asset://beat", { bands: 8, fps: 30 })`.
The prepared object provides `level(t)`, `band(index, t)`, `onset(t)`, `beatPhase(t)`, and `beatConfidence(t)`;
its numeric tables are frozen into the compiled artifact and are read without decoding at
frame time. Bind the declared audio control with `--asset beat=beat.wav`.
`beatPhase` interpolates between tracked beat times, including fractional frame periods.
When periodic support is below 0.35 or fewer than three beats are tracked, it returns zero
and compilation emits an `audio-beat-uncertain` warning. `beatConfidence(t)` returns the
whole-asset periodic support in 0–1; use `onset(t)` for irregular individual events.
Studio downloads the same frozen FFmpeg 48 kHz mono PCM used by CLI. Its compiler Worker
keeps decoded PCM and analysis tables across source edits; browser audio decoders are not used.

### Structured data example

Save as `bars.motion.tsx`:

```tsx
export const controls = {
  data: {
    rows: array(record({
      id: string(), label: string(), value: number({ min: 0, max: 100 }),
    }), { minItems: 1, maxItems: 5, key: "id" }),
  },
};

export default function Bars(ctx, props, data) {
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
valle motion check bars.motion.tsx --data bars.json
valle motion render bars.motion.tsx --data bars.json --frame 45 -o bars.png
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
export const controls = {
  assets: { poster: asset({ kind: "image", required: true }) },
};
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
valle motion check poster.motion.tsx --asset poster=poster.png
valle motion render poster.motion.tsx --asset poster=poster.png --frame 0 -o poster-frame.png
```

Media primitives default to block layout. A direct `Image` child of rich `Text`
defaults to an atomic inline box; elsewhere, use `display: "inline"` to place
media in a text line. Authored display styles or utilities override these defaults.
Asset kinds are `image`, `audio`, `video`, `font`,
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
`--font fonts/Brand.ttf` registers a preferred face by its own family name for render
and Studio. Repeat `--font` to compose an ordered font stack, and use the same
options for `motion check`. The built-in Noto/serif/monospace palette provides
fallback; packages include the selected families, weights and script coverage
instead of always copying the entire palette. Dynamic text retains wider fallback
coverage for later frames.
Resource font references belong in `style.fontFamily`; a font-family utility such as
`[font-family:'asset://brand']` is rejected because utilities do not bind assets.

Web hosts can supply an ordered `fonts` list of TTF/OTF URLs or bytes when creating
`valle-engine` players. Fonts load before layout and produce a new render identity.
They do not need to match built-in Noto bytes. See the [SDK font options](../web/packages/engine/README.md#fonts).
Formula faces and explicit `asset://` font bindings remain resources of the work.

## Time and animation

### Context fields

| Expression | Meaning |
| --- | --- |
| `ctx.localFrame` | Clamped zero-based source frame |
| `ctx.seconds` | Exact mapped source sample time in seconds, including subframes |
| `ctx.progress` | `clamp(sourceTime × actualFPS / durationFrames, 0, 1)` |
| `ctx.durationFrames` | Composition duration quantized at the actual FPS |
| `ctx.fps.num`, `ctx.fps.den` | Rational frame-rate numerator/denominator |
| `ctx.viewport.width`, `.height` | Logical canvas dimensions in pixels |
| `ctx.unit.index`, `.count`, `.start`, `.end` | Only inside `Text perUnit`; start/end are text byte offsets |

Define animation windows explicitly with `ctx.seconds`, sequences and expressions.
The last displayed source frame precedes the quantized duration boundary.

### Time functions and visibility evaluation

Motion's time-function intermediate representation recognizes bounded numeric
expressions that are affine in one time input: `ctx.seconds` or `ctx.localFrame`.
It accepts finite constants, addition, subtraction, negation, multiplication by a
time-independent expression, and division by a finite nonzero time-independent
expression. Mixed time inputs, nonlinear terms, and unsupported operations stay
on the ordinary expression evaluator.

Each recognized function stores its mathematical slope and offset for analysis.
Runtime evaluation keeps the original arithmetic tree and operation order, so
floating-point rounding at a visibility threshold is unchanged. The derivative
is with respect to the selected input; it is not yet a screen-space velocity or
an automatic motion-blur parameter.

The internal `range_on(start, end)` operation evaluates an inclusive numeric
enclosure through that same tree. Every arithmetic bound rounds outward; overflow
or a division whose enclosure crosses zero leaves the range unproven. A comparison
is certified constant only when the entire enclosure lies on one side of its
threshold. For authored compositions with a fixed default frame rate, leaf
activation precomputes certificates over the full composition time domain. The
certificate is used only when the requested sample, frame rate, and duration
belong to that domain and the sample's projected seconds lie inside the proved
numeric interval. Requests outside it evaluate the gate directly.

If a comparison changes somewhere during the composition, activation inspects
bounded binary subdivisions (at most eight splits along a branch and 31 visited
intervals per leaf). It retains only proven intervals and merges adjacent intervals
with the same result. Samples in unresolved gaps, including threshold crossings,
evaluate the original gate. This works for both composition seconds and local
frames without quantizing subframe requests. The selected expression roots then
run against a dense arena-indexed value table.

Absolute-leaf visibility gates and [instance-row range gates](#automatic-instances-for-prepared-maps)
consume this representation. Comparisons work with the time expression on either
side, and instance ranges use a binary search over row indices. Structurally
identical leaf time functions share one result within a sample. Each Shutter or
Echo sample gets a new result cache, including when it carries the same local
frame number. Non-finite values or unrecognized expressions fall back to full
evaluation.

The compiler lowers [TimeScope](#time-scopes) time reads through the authored
nested offset/speed chain. Because TimeScope adds no layout box, independent
absolute leaves below it can use the same certified visibility gate; the
unresolved threshold still evaluates the original expression at the exact sample.

This representation does not yet provide general differentiation, time shifting,
interval proofs for other node kinds, screen-space bounds, or SIMD evaluation.

### Interpolation and springs

`interpolate(input, inputRange, outputRange, { easing, colorSpace, hue }?)` clamps outside its
input range. Ranges must have equal length ≥ 2, strictly increasing finite input
stops, and compatible output types. Stops are normally known at compile time: literals or
immutable constant arrays. One exception is the same numeric shift added to every input stop,
such as `interpolate(ctx.seconds, [ctx.unit.index * 0.1, 1 + ctx.unit.index * 0.1], [0, 1])`.
The compiler subtracts that shift from the input. Different shifts, spreads and holes are
rejected.

Numbers, colors, lengths, angles and supported geometric values interpolate;
boolean/string/enum values are discrete. Keep corresponding length/angle units
compatible. For a destination depending on a prop or viewport, interpolate a
normalized number and multiply afterward.

Color interpolation uses `colorSpace: "srgb"` by default. Choose `"linear"`,
`"oklab"`, or `"oklch"` for linear sRGB, OKLab, or OKLCH interpolation.
For OKLCH, `hue` can be `"shorter"` (default), `"longer"`, `"increasing"`,
or `"decreasing"`; it is invalid in other color spaces. Authored colors stay
floating point through expression evaluation and DrawProgram emission for solid
paints, Path gradient stops, Text fills, and `perUnit.color`. For example:

```tsx
backgroundColor: interpolate(ctx.progress, [0, 1], ["#2140ff", "#ffd000"],
  { colorSpace: "oklch", hue: "shorter" })
```

Easing names: `linear` (default), `ease`, `easeIn`, `easeOut`, `easeInOut`, `exp`,
`easeInBack`, `easeOutBack`, `easeInOutBack`, `elastic`, `bounce`,
`steps(n, start|end)`, and `cubic-bezier(x1,y1,x2,y2)` with finite coordinates and
x coordinates in `[0,1]`. Back curves and the decaying elastic-out curve can
overshoot; `bounce` uses a piecewise quadratic bounce-out curve. `steps(n)` defaults
to `end`, with a positive integer step count. `start` jumps to the first step at
the segment's first input, including the first frame; `end` jumps at each interval's
end. Both finish at the destination. `easing` can be one string or an array with
one entry per segment, including immutable constants. Custom function easings
are not yet admitted. Other option names, including
`extrapolateLeft`/`extrapolateRight`, are not author options in this CLI language.

Save as `spring.motion.tsx`:

```tsx
export const composition = { width: 1920, height: 1080, fps: 30, duration: 5 };
export default function SpringTitle(ctx) {
  const enter = spring({ elapsedFrames: ctx.seconds * ctx.fps.num / ctx.fps.den, fps: ctx.fps, preset: "gentle" });
  const exit = interpolate(ctx.seconds, [4.4, 4.8], [1, 0]);
  return (
    <Scene className="h-full w-full flex items-center justify-center"
      style={{ backgroundColor: "#0f172a" }}>
      <Text style={{ fontSize: 44, color: "#f8fafc", opacity: clamp(enter, 0, 1) * exit,
        translate: point(0, (1 - enter) * 50) }}>Spring into motion</Text>
    </Scene>
  );
}
```

`spring({ elapsedFrames, fps: ctx.fps, preset })` starts at 0 with zero velocity by default
and converges to 1; it can overshoot. It continues settling after its authored window when
given `elapsedFrames`. Presets: `gentle`, `wobbly`, `stiff`, `slow`, `bouncy`.
Alternatively provide static `mass`, `stiffness`, `damping` (defaults 1, 100, 10).
Mass/stiffness must be positive; damping must be non-negative. A preset and physical
parameters are mutually exclusive. There are no `from`, `to`, `duration` or
`overshootClamping` options: scale/offset/clamp the result explicitly.

Optional `initialVelocity` is a finite, compile-time number in **normalized distance
per second**, default 0. It can accompany a preset. For a segment moving from 100px
to 300px with an incoming velocity of 100px/s, use `initialVelocity: 0.5`
(`100 / (300 - 100)`) and position `100 + 200 * spring(...)`. This preserves the
incoming velocity at that authored boundary. It does not infer an initial condition
from playback history or automatically smooth arbitrary keyframe joins.

`springVelocity({ elapsedFrames, fps: ctx.fps, ... })` accepts the same options and
returns the **analytic derivative per second** of the normalized spring position.
Multiply by the travel distance to obtain px/s. Before time zero both position and
velocity are zero; at zero the velocity equals `initialVelocity`. Neither function
uses iterative simulation or a settling threshold, including for zero damping.

```tsx
const p = spring({ elapsedFrames: ctx.localFrame, fps: ctx.fps, preset: "gentle" });
const v = springVelocity({ elapsedFrames: ctx.localFrame, fps: ctx.fps, preset: "gentle" });
// For a 120px horizontal movement:
// translate: point(120 * p, 0)
// motionBlur: motionBlur(point(120 * v * ctx.fps.den / ctx.fps.num, 0), 180)
```

For an authored state update rather than the analytic spring, bake it once at prepare time:

```tsx
const motion = simulate({
  dt: 1 / 240, duration: 2,
  init: () => ({ x: 100, velocity: 0 }),
  step: (state, time, dt) => {
    const velocity = state.velocity + ((300 - state.x) * 40 - state.velocity * 4) * dt;
    return { x: state.x + velocity * dt, velocity };
  },
});
// In the component: left: motion.at(ctx.seconds).x
```

`simulate({ dt, duration, init, step })` runs `step` at fixed `dt` increments in the
prepare-time sandbox. The state has 1–16 named numeric fields; `step` must return the
same fields with finite values. The table is limited to 16,384 steps and 131,072 values.
At frame time, `.at(t).field` clamps `t` to `[0, duration]` and linearly interpolates
the two neighboring samples, including at fractional times. Each lookup reads the
baked table, so rendering frames out of order has the same result as sequential playback.
Direct module-level simulations whose callbacks use only numeric literals, their
parameters, local `const`s and state fields are cached by content hash while Studio
recompiles. A callback that calls helpers, captures other values or writes external
state still runs during each prepare pass.

Compose separate curves for opacity, position, scale and path reveal. An easing
per segment does not guarantee matching velocities across adjacent segments.

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
| `trail(progress, index, { gap, mode? })` | Sample `progress + index × gap`; negative gap places later indices behind the head. Mode `clamp` (default) or `wrap` |
| `wiggle(frame, ctx.fps, { seed, frequency, amplitude, phase? })` | Seeded scalar displacement; frequency in seconds, non-negative amplitude |
| `wiggle2D(frame, ctx.fps, { seed, frequency, amplitude: [x,y], phase? })` | Seeded point displacement; suitable for `style.translate` |

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

Write style objects with camelCase keys; quoted kebab-case keys are also accepted.
For example `backgroundColor` and `"background-color"` name the same property.
Immutable object references, aliases, nested object spreads, and prepare-time
string computed keys are supported. Local objects can contain frame expressions
as long as their property set is fixed:

```tsx
const cardStyle = { padding: 24, borderRadius: 20, backgroundColor: "#123456" };
export default function Demo(ctx) {
  const style = { ...cardStyle, width: 240 + ctx.localFrame, height: 120 };
  return <Scene><View style={style} /></Scene>;
}
```

Objects retain their declaration scope. Duplicate keys replace the value while
keeping the first insertion position, following JavaScript object enumeration;
this also applies across spreads. Getters, setters, methods, prototype changes,
and frame-varying property names/sets are rejected. Style object references must
currently resolve to authored object literals; arbitrary helper-returned objects
are not a supported style input. The same rules apply to `Span`'s supported subset.

`className` supports static strings and finite conditional/template choices.
Ordinary inline styles override ordinary utilities; an
important utility (trailing `!`) overrides an ordinary inline style. Utility
conflicts use Tailwind rule order rather than class-string order.
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

For geometry that follows the logical canvas, use `ctx.viewport.width`/`.height` in numeric expressions.
For percentage translations use a typed pair such as `"-50% -50%"`.
When interpolating lengths, keep the units the same at corresponding stops.

A value that has to be shared is a `const` at the top of the file, not a variable:
author CSS custom properties (`style={{ "--accent": … }}`) and variable consumers
(`var(--accent)`, `w-(--size)`, `text-(length:--size)`) are permanently rejected, and the
diagnostic names the `const` replacement. The internal `--tw-*` composition slots belong
to the utility lowerer and are not author API.

Frame-varying values are expressions, not variables. Write the expression where the value
is used; a helper may take the value as a parameter and keep the expression in one place.

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
| `mixBlendMode` | `normal`, `plus-lighter`, `multiply`, `screen`, `overlay`, `darken`, `lighten`, `color-dodge`, `color-burn`, `hard-light`, `soft-light`, `difference`, `exclusion`, `hue`, `saturation`, `color`, `luminosity` | S; combines with the already-painted backdrop |
| `mixBlendSpace` | `"srgb"` (default), `"linear"` | S/D finite choices; selects encoded or linear sRGB for this node's creative blend; not inherited |
| `isolation` | `auto`, `isolate` | S; establish a compositing boundary when needed |
| `objectFit`, `objectPosition` | `contain`, `cover`, `fill`, `none`, `scale-down`; `"center"` | S for media placement |
| `imageRendering` | `auto`, `pixelated` | S; choose sampling appropriate to the image |

Avoid `groove`, `ridge`, `inset`, `outset` **border styles**; they are not emitted.
This restriction does not refer to an inset box shadow. Explicit `borderWidth`
without a non-empty border style is not a way to draw a border.
Dashed/dotted/double borders currently require uniform widths/colors around the
box; unequal per-edge paint supports solid/none/hidden styles.

CSS gradients default to OKLab interpolation. Linear, radial, conic and repeating
gradients accept `in oklab`, `in oklch`, `in srgb`, and `in srgb-linear`.
OKLCH accepts `shorter hue` (default), `longer hue`, `increasing hue`, and
`decreasing hue`, for example `linear-gradient(90deg in oklch longer hue, red, blue)`.
Interpolation uses premultiplied alpha; polar hue itself is not premultiplied.
Native and browser rendering share floating-point color-stop lowering. CSS stop
colors still resolve through the parser's 8-bit sRGB representation; this does not
provide a float/wide-gamut author color pipeline. Other interpolation spaces such
as `in display-p3` and `in lab` produce explicit diagnostics.
Multiple gradient layers use a comma-separated `backgroundImage` value.
Gradient stops currently must resolve inside the emitted gradient's `[0, 1]`
offset range. CSS stops outside that range can fail DrawProgram validation;
extending/remapping the gradient rather than clamping its stops remains unimplemented
([CSS gradient stop positions](https://www.w3.org/TR/css-images-3/#linear-gradient-syntax)).

### Transforms

| Property | Example / contract |
| --- | --- |
| `translate` | `point(x,y)`, `"12px 24px"`, `"-50% -50%"`; a single CSS length defaults Y to zero; typed values and CSS templates can vary per frame |
| `rotate` | `"15deg"`, angle-valued interpolation, or a CSS template with an explicit angle unit |
| `rotateX`, `rotateY` | Numeric degrees or typed angles for a projected layer |
| `scale` | `1.2`, `point(1.2,0.8)`, or CSS `"120% 80%"`; `"none"` resets the property |
| `scaleX`, `scaleY` | One axis at a time; compiler lowers to a 2D scale binding |
| `transformOrigin` | `"50% 50%"`, `point(x,y)`; defaults to center |
| `transform` | Ordered 2D translate/rotate/scale/skew/matrix list, closed templates and finite branches; admitted 3D forms use the projection adapter |
| `perspective`, `perspectiveOrigin` | Positive perspective distance and static/dynamic supported origin components |
| `transformStyle`, `backfaceVisibility` | Explicit `transformStyle` must be static `"preserve-3d"`; backface visibility is static `"visible"` / `"hidden"` |

For animated 2D translation, use `translate: point(x,y)` or the CSS list
`` transform: `translate(${x}px, ${y}px)` ``. Transform functions use CSS comma
separators; independent properties use spaces. Single-argument `translate(12px)`
means `translate(12px, 0px)`. Percentage translations use the element's border box.

Independent `translate`, `rotate`, and `scale` accept the same closed CSS templates
and finite conditional branches as the ordered `transform` list. Templates can be
stored in local constants, returned from helpers, and passed as component props:

```tsx
const move = `${ctx.seconds * 100}px 10%`;
const turn = ctx.seconds < 1 ? "none" : `${ctx.seconds * 90}deg`;
const zoom = `${1 + ctx.seconds * 0.1} 1`;
return <View style={{ translate: move, rotate: turn, scale: zoom,
  transform: `skew(${ctx.seconds * 10}deg, 0deg)` }} />;
```

Skew uses the ordered list's `skew`/`skewX`/`skewY` functions. Numeric template
holes need the property's CSS units (`px`, `%`, `deg`, etc.); typed angles and
lengths already carry units. Arbitrary string holes cannot inject CSS structure:
write `` rotate: `${condition ? 0 : 45}deg` `` or branch between whole CSS values.
Wrong value types report the authored expression and an accepted replacement.

Dynamic independent transforms with fixed presence and priority retain geometry
reuse. A time-dependent choice between `none` and a transform, inherited values,
or a change of `!important` rebuilds layout so positioned descendants use the
correct containing block. Point values acquire pixel units when used by
`translate`/`transformOrigin`; the same point remains unitless when used by `scale`.

Independent properties compose in the order **translate → rotate → scale →
transform list**, around `transformOrigin`, regardless of style-object key order.
The list retains every function in authored order: `scale(2) translateX(10px)`
translates by 20px; `translateX(10px) scale(2)` translates by 10px. Combining an
independent scale with a list scale is valid and does not require nested boxes.
`scale: 1`, `translate: "0px"` and an identity matrix still establish a containing
block for positioned descendants; `none` does not. Singular transforms paint no
subtree, while its layout boxes remain available.

2D functions include `translate`, `translateX/Y`, `rotate`, `scale`, `scaleX/Y`,
`skew`, `skewX/Y`, and `matrix(a,b,c,d,e,f)`. Supported length `calc()` expressions
retain their layout reference box. All branches of a conditional list are checked,
and templates accept typed numeric/length/angle holes with valid CSS units. Every
frame is evaluated directly; selecting a different list never starts a transition.
Independent transforms and lists depending on `bounds`/`anchor`/`project3d` use syntax-valid probes prepared
before rendering. Every branch must preserve containing-block presence: different
non-empty lists are allowed, as are branches that all reset to `none`/`initial`,
but switching between a list and `none` is rejected. Inherited presence is not
admitted in these post-layout expressions, and `!important` must stay the same
across branches. The final values are evaluated from
the requested frame's layout, without using previously rendered frames.

The separate CSS 3D adapter supports ordered `translateZ`, `translate3d`,
`rotateX/Y/Z`, `rotate3d`, `scaleZ`, `scale3d` and its admitted translate/rotate/scale
2D functions. A mixed 3D list does not yet accept skew/matrix or nested length math.
Use the separate `perspective` style rather than a `perspective()` transform function.
`matrix3d` remains unsupported. A 2D list's support does not establish full CSS 3D
flattening or composition semantics.
Native raster currently rejects spatial blur/drop-shadow combinations whose final
affine transform produces a non-axis-aligned Gaussian covariance (for example,
skew plus blur). This is an explicit backend limitation, even when layout and
DrawProgram generation accept the individual properties.
`projectQuad` is explicitly rejected as an author API.

### Filters and animated paint example

Supported CSS filter functions are `blur`, `brightness`, `contrast`, `grayscale`,
`hue-rotate`, `invert`, `opacity`, `saturate`, `sepia`, `drop-shadow`.
They work in `filter` and `backdropFilter` through Motion's filter lowering.
Filters compose in written order. `filter` affects the node and descendants;
`backdropFilter` samples already-painted content behind the node.

`filter: "chromatic-aberration(6px)"` is also available as a standalone
node filter. It shifts red and blue by 3 px in opposite directions while green
stays centered. The signed offset must be finite and within ±256 px. It is
currently a standalone `filter` value; chaining it with other CSS filters and
using it in `backdropFilter` are not admitted.

`filter: "radial-blur(40px 20px 16px)"` blurs the node and its descendants along
rays from a center at local `(40, 20)` px. The last length is the maximum sample
displacement in either direction along each ray (0–128 px). It uses a shared working-linear RGBA16F approximation with geometrically increasing
three-tap passes, so cost grows logarithmically with device radius. Near the center,
a bounded exact kernel avoids rings and excess diffusion; samples never cross the
center. Native and CanvasKit use the same shader, including at enlarged output sizes. The value is
standalone; it cannot be chained with CSS filters or used in `backdropFilter`.

`filter: "film-grain(7 0.12 2px)"` adds seeded monochrome grain in sRGB display encoding to the node and its descendants. The unsigned integer seed fixes the sequence;
amount is in 0–1 and grain size in 1–64 local px. Noise changes once per local frame and stays deterministic across ROI crops and
random frame seeks. Noise amplitude follows each channel's distance to black and white, preserving
both endpoints without asymmetric clipping. Display-gray mean brightness remains
stable at high amounts. The result returns to working-linear light, scales with
source alpha, and does not alter alpha. This is a standalone `filter` value.

These advanced filters use the same cascade as ordinary CSS filters, including
important utilities. Template strings and conditional expressions can animate parameters,
for example ``filter: `glow(${ctx.progress * 24}px 1 #22d3ee)` ``. Each frame must
satisfy the parameter bounds below; effects cannot be combined in one filter list.
The compiler checks numeric ranges it can establish for template parameters and
transition bindings. A range wholly outside the domain is an error; a range that
might leave it produces `parameter-range`. Unknown expressions remain runtime-checked.
Runtime diagnostics identify the parameter, value and frame; CLI authoring errors
also resolve the expression to its source location. Use `clamp()` explicitly when
clipping an overshoot is intended; the renderer never silently clamps it.

`filter: "lens-distortion(-0.3 0.2)"` resamples the node and its descendants around
the center of their uncropped frame. The two unitless radial coefficients are each
within ±0.5; negative values pull source samples toward the center, making content
appear farther from it. Pixels outside the source are transparent, and the effect
stays inside the original frame. The frame and coefficients remain stable across
ROI crops. This standalone value requires a similarity device transform.

`filter: "glow(24px 1.5 #22d3ee)"` adds a tinted halo behind the node and its
descendants while retaining the source. It uses the same multilevel F16 pyramid
as scene bloom, driven by the subtree alpha. The radius controls the halo extent
in 0–128 px; intensity is in 0–4. This requires a standalone `filter` value and is not admitted in `backdropFilter`.

`Scene bloom={{ threshold: 0.8, intensity: 1.2, radius: 48 }}` extracts bright
pixels from the whole scene and adds a soft halo through a five-level F16
downsample/upsample pyramid. `threshold` is in 0–1, `intensity` in 0–4, and
`radius` in 0–128 px. The optional `knee` is in 0–1 and defaults to 0.1. These
values must be static; the effect runs after the scene's children are composed.

Use non-negative blur lengths, finite scalar/percentage factors and explicit
angle units for hue rotation. `drop-shadow` uses offsets, optional blur and color;
it is different from a spread/inset `boxShadow`. SVG `url(...)` filters are not
supported. `none` overrides lower-priority filters. Zero blur keeps CSS cascade
and containing-block semantics even though it does not soften pixels. Filter
`blur()` and the third length of `drop-shadow()` are Gaussian sigma, unlike the
blur radius of `box-shadow`/`text-shadow` ([CSS Filter Effects](https://www.w3.org/TR/filter-effects-1/#funcdef-filter-blur)).
Post-layout filter templates and branches follow the same presence rule as
transform lists. Probe values do not bypass per-frame validation: a negative blur
computed from actual bounds still fails on that frame.

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

Motion validates every candidate in `className` against its supported catalog.
There is no config/plugin processing, unbounded class generation or implicit
interaction state. Responsive variants use the logical canvas; finite choices
may also depend on `ctx` or props:

```tsx
<View className={`p-4 ${ctx.localFrame >= 30 ? "w-40" : "w-20"} bg-${active ? "blue-500" : "red-500"}`} />
```

String constants, ternaries, finite template holes, local bindings and pure helpers
that lower to those expressions are supported. Every candidate is checked before
rendering, including inactive branches. Independent whitespace-separated choices
remain separate selectors rather than combinations of entire style objects.
One token can have at most 64 finite choices; arbitrary numeric/string substitutions
such as `` `w-${ctx.localFrame}` `` are rejected. Selection is discrete and does
not start an implicit transition or animation. Post-layout bounds cannot control
class selection, since classes can affect layout.

Variants are permanently rejected: `sm:`/`md:`/`lg:`/`xl:`/`2xl:`, `min-*`, `max-*`,
`portrait:`, `landscape:`, `dark:`, `hover:`, `focus:` and `press:`. The canvas is fixed by
the entry file's `composition` for standalone preview, so a breakpoint would be a constant.
Keep one file per deliverable shape, or branch on props or data. Do not compare
`ctx.viewport.width` as if it were a CSS breakpoint.

`ctx.viewport.width`/`.height` are numeric expressions for geometry (bar widths, positions).
They read the Motion composition's logical canvas in standalone and Timeline use.
Timeline fits that completed canvas into the clip target rectangle. They are not the
display window, `--output-size`, or system preferences. The low-level Rust
`LayoutOptions.viewport` takes device-pixel extents: its logical canvas is `size / DPR`.
Normal Native/Wasm hosts use DPR 1 and scale delivery afterward. The same rules prepare
every finite class choice, and class string order still does not decide a conflict.

The following catalog families are admitted. `N` means a finite non-negative
number unless a narrower range is stated; it does not imply arbitrary CSS text.

| Family | Admitted forms |
| --- | --- |
| Display / position | `block`, `inline`, `inline-block`, `flex`, `inline-flex`, `grid`, `inline-grid`, `hidden`; `static`, `relative`, `absolute`, `fixed` |
| Box / visibility | `box-border`, `box-content`, `visible`, `invisible`, `isolate`, `isolation-auto` |
| Flex | `flex-row`, `flex-col`, reverse variants; `flex-wrap`, `flex-wrap-reverse`, `flex-nowrap`; `flex-auto`, `flex-initial`, `flex-none`, `flex-N`; `grow`, `grow-N`, `shrink`, `shrink-N` |
| Alignment | `items-{baseline,center,end,start,stretch}`; `self-{auto,baseline,center,end,start,stretch}`; `justify-{around,between,center,end,evenly,normal,start,stretch}`; `content-{around,between,center,end,evenly,normal,start,stretch}` |
| Spacing | `gap`, `gap-x/y`; `m`, `mx/y`, `mt/r/b/l/s/e`; `p`, `px/y`, `pt/r/b/l/s/e`; `inset`, `inset-x/y`, `top/right/bottom/left`, each followed by `-N`, `-px`, `-full` |
| Automatic / negative spacing | `auto` for margins/insets; a leading `-` for margins/insets, not padding/gap |
| Sizing | `w`, `h`, `min-w/h`, `max-w/h`, `size`, `basis` followed by `-N` or `-{auto,px,full,screen}`; fractions on `w/h`, `min-w/h`, `max-w/h`, and `basis`; container tokens `max-w-{3xs,2xs,xs,sm,md,lg,xl,2xl,3xl,4xl,5xl,6xl,7xl}` |
| Aspect | `aspect-auto`, `aspect-square`, `aspect-video` |
| Grid | `grid-cols/rows-N`, `col/row-span-N`, `col/row-start/end-N`, N 1–64; `grid-cols/rows-none`, `col/row-span-full`, `col/row-start/end-auto` |
| Grid flow / automatic tracks | `grid-flow-{row,col,dense,row-dense,col-dense}`; `auto-cols/rows-{auto,fr}` |
| Overflow | `overflow-{clip,hidden,visible}`; scrolling/auto overflow is not implemented |
| Text size / alignment | `text-{xs,sm,base,lg,xl,2xl,3xl,4xl,5xl,6xl,7xl,8xl,9xl}`; optional numeric or arbitrary leading modifier, such as `text-sm/6` and `text-[13px]/[20px]`; `text-{left,center,right,justify,start,end}` |
| Font weight / style | `font-{thin,extralight,light,normal,medium,semibold,bold,extrabold,black}`; `italic`, `not-italic` |
| Font family | `font-sans`, `font-serif`, `font-mono`; family stacks resolve only against fixed fonts supplied to the package |
| Tracking / leading | `tracking-{tighter,tight,normal,wide,wider,widest}`; `leading-N`, `leading-{none,tight,snug,normal,relaxed,loose}` |
| Text behavior | `uppercase`, `lowercase`, `capitalize`, `normal-case`; `break-{all,keep,normal}`; `whitespace-{normal,nowrap,pre,pre-line,pre-wrap}`; `truncate`, `text-clip`, `text-ellipsis`, `tabular-nums` |
| Text decoration / clamp | `underline`, `overline`, `line-through`, `no-underline`; `line-clamp-N` for 1–64 and `line-clamp-none` |
| Radius | `rounded`, `rounded-{none,xs,sm,md,lg,xl,2xl,3xl,4xl,full}`; edge/corner prefixes `rounded-t/r/b/l/tl/tr/br/bl-...` |
| Border / outline | `border`, `outline`; `border-{solid,dashed,dotted,double,none}`, same outline styles; width `border-N`, side/axis width variants, `outline-N`, `outline-offset-N` |
| Shadows | `shadow`, `shadow-{2xs,xs,sm,md,lg,xl,2xl,none}` |
| Opacity | `opacity-N`, N 0–100 |
| Gradients | `bg-linear-to-{t,tr,r,br,b,bl,l,tl}`, `bg-radial`, `bg-conic`; `from/via/to-COLOR` and `from/via/to-N%` |
| Colors | `bg`, `text`, `border`, border side/axis variants, `outline`, `decoration`, `shadow`, `text-shadow` with admitted color tokens |
| Additional alignment | `justify-items-*`, `justify-self-*`, `place-items-*`, `place-content-*`, `place-self-*`, with the corresponding supported CSS keyword |
| Order / stacking | `order-N`, `-order-N`, `order-none`; `z-N`, `-z-N`, `z-auto` |
| Media | `object-{contain,cover,fill,none,scale-down}`; `object-{center,top,right,bottom,left,left-top,left-bottom,right-top,right-bottom}` |
| Text wrapping | `text-wrap`, `text-nowrap`, `text-balance`, `text-pretty` |
| 2D transforms | `translate`, `translate-x/y` with spacing, fractions, `full`, `px`, or arbitrary lengths; `rotate-N`, `scale-N`, `scale-x/y-N` with integer N; `origin-{center,top,right,bottom,left,top-left,top-right,bottom-left,bottom-right}`; negative transforms; `translate-none`, `rotate-none`, `scale-none` |
| Filters | `blur`, `blur-{xs,sm,md,lg,xl,2xl,3xl,none}`; `brightness-N`, `contrast-N`, `grayscale-N`, `hue-rotate-N`, `invert-N`, `saturate-N`, `sepia-N` with non-negative integer N; bare `grayscale`, `invert`, `sepia`; negative hue rotation; `drop-shadow`, `drop-shadow-{xs,sm,md,lg,xl,2xl,none}`; `filter`, `filter-none` |
| Backdrop filters | `backdrop-` versions of blur/brightness/contrast/grayscale/hue-rotate/invert/saturate/sepia; `backdrop-opacity-N`; `backdrop-filter`, `backdrop-filter-none` |

Color tokens are `black`, `white`, `transparent`, `current`, or a palette family
plus shade. Families: slate, gray, zinc, neutral, stone, red, orange, amber, yellow,
lime, green, emerald, teal, cyan, sky, blue, indigo, violet, purple, fuchsia, pink,
rose. Shades: 50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950. Palette colors
can use `/N` opacity, for example `bg-blue-500/20`.

Numeric spacing uses the utility scale (for example `p-4` is 16px with the default
root sizing), not literal pixel counts. Fractions such as `w-1/2` and `basis-1/3`
resolve to percentages. `flex-1` uses CSS `flex: 1`, including its percentage basis.
Arbitrary values are admitted on `w/h`, `size`, `min-w/h`, `max-w/h`, `basis`,
all admitted padding/margin axes and sides, `gap`, `gap-x/y`, `inset`, `inset-x/y`,
`top/right/bottom/left`, `grid-cols/rows`, `aspect`, `bg` (color), `leading`, and
`text` (color or size). Examples: `w-[320px]`, `w-[calc(100%_-_40px)]`,
`grid-cols-[1fr_2fr]`, `aspect-[4/3]`, `bg-[#123456]`, `px-[13px]`,
`-left-[20px]`, `leading-[1.4]`, and `text-[length:13px]/[20px]`.
Negative arbitrary margins/insets are supported; negative padding/sizes are rejected.
Slash leading uses the spacing scale (`/6` = `1.5rem`); a bracket value keeps its
CSS units or unitless multiplier. For width-constrained standalone text, select a
block/flex/grid layout context: inline text does not acquire a width from `width`.
Visual arbitrary values include `translate-x-[25%]`, `-rotate-[0.25turn]`,
`scale-[1.25_0.5]`, `origin-[20%_40%]`, `blur-[3px]`, `brightness-[1.25]`,
`drop-shadow-[2px_4px_3px_#000]`, `filter-[blur(3px)_brightness(1.25)]`, and
`backdrop-filter-[blur(3px)]`. Values go through the same strict CSS parser as
inline styles.

General arbitrary properties use canonical CSS names: `[width:123px]`,
`[padding:3px_7px]`, `[border:2px_solid_red]`,
`[transform:scale(1.2)_translateX(4px)]`, and `[backdrop-filter:blur(2px)]`.
They use the same property admission, value checks, cascade and cache rules as
inline CSS; this syntax does not enable unsupported properties or values.
All candidates in a finite conditional class are validated, including branches
not selected at the requested frame. Append `!` for importance, for example
`[width:123px]!`.

Underscores represent spaces in arbitrary values; escape an underscore to retain
it in a font name. Inside a JavaScript string, escape the backslash too:
`className={"[font-family:'KaTeX\\_Main']"}`. Quoted strings, nested parentheses
and math operators are decoded before CSS parsing. Within utility values, URL
backgrounds, custom-property declarations, and unsupported math
functions remain rejected. Static inline CSS variables use the preparation path
described above.
Finite font-family class choices can select among fixed fonts supplied to the
render package; each frame resolves glyphs and layout for its selected family.
Use `style={{ fontFamily: "asset://brand" }}` for a bound font resource.
Font names do not load system or remote fonts.
The default pack provides Noto Sans, KaTeX Main serif faces and Noto Sans Mono for
the three font-family utilities, plus fixed script/emoji fallbacks. Finite choices
retain the required candidate families in the render package, including inherited
font classes on ancestor containers.

Translation percentages use the node's own border box. Transform axes compose
within each property; scale arbitrary values set the whole scale property.
Individual filter utilities compose in a fixed order: blur, brightness, contrast,
grayscale, hue rotation, invert, saturate, sepia, drop-shadow. Backdrop chains put
opacity after invert and omit drop-shadow. A whole `filter-[...]` retains the
authored function order. Composition slots do not inherit to child nodes.
Conflicting utilities still follow rule order: for example, `blur-sm` wins over
`blur-none`; use `filter-none` or an appropriate important override to reset the
effect. Negative/non-finite filter values fail; mixed-unit blur expressions are
also checked after resolving the actual frame's sizing context.

Utilities are expanded once during prepare, in a fixed property order. Reordering
`className` does not change conflicting utility results.
Ordinary inline styles override ordinary utilities; a trailing `!` marks an
important utility, for example `w-40!`, which overrides ordinary inline width.
Motion primitives default to `box-sizing: border-box`, zero margin/padding, and
zero-width solid borders. `border-2 border-white` paints directly; `box-content`
and `border-none` explicitly override those defaults.
Image and video block defaults have lower priority than authored display rules.
An image directly inside rich `Text` defaults to an atomic inline box when no
authored display choice applies at that frame. Explicit inline media retains its
own width/height, and `bounds()` uses its actual placement in the text flow.

Forbidden forms include `hover:...`, `dark:...`, every other variant, leading `!`,
every `animate-*` class, `transition-*`, `transform-gpu` and `transform-cpu`. Drive
loops with `interpolate` over local time. A legal utility
name still needs meaningful values and an appropriate node/layout context.
Intrinsic sizing (`w-min`, `w-max`, `w-fit` and the corresponding height/min/max
variants), `auto-cols-min/max`, and `auto-rows-min/max` are permanently rejected: write
an explicit length or percentage, or `measureText` for a text-sized box. Inline
`min-content`, `max-content`, and `fit-content` are rejected for the same reason. Grid track lists
must be fully valid: `subgrid` and trailing unknown tokens fail during compilation
or artifact validation, rather than becoming empty or truncated track lists.

## Text, fonts and formulas

### Typography styles

| Properties | Values / behavior |
| --- | --- |
| `fontFamily` | Static family string or bound `asset://fontName`; cannot vary by frame |
| `fontSize`, `fontWeight`, `fontStyle` | Size in pixels/lengths; continuous numeric weight (including fractions); normal/italic/oblique as accepted by the font |
| `lineHeight` | Number is a multiplier (`1.4`); length string is an explicit height (`"32px"`) |
| `letterSpacing`, `wordSpacing` | Lengths; explicit numeric pixel values are useful for spacing |
| `textAlign`, `direction` | left/right/center/justify/start/end; ltr/rtl |
| `whiteSpace` | normal/nowrap/pre/pre-wrap/pre-line; use an explicit string containing `\n` for source line breaks |
| `wordBreak`, `overflowWrap` | normal/break-all/keep-all; normal/break-word/anywhere where accepted |
| `textTransform` | none/uppercase/lowercase/capitalize |
| `textOverflow`, `lineClamp` | clip/ellipsis; positive line clamp, usually with a constrained width and overflow |
| `textDecoration`, `textDecorationLine/Style/Color/Thickness` | underline/overline/line-through, color/width; decoration style currently supports only `solid` |
| `textShadow` | CSS shadow list, for example `"0px 2px 8px #00000080"` |
| `WebkitTextStroke`, `WebkitTextStrokeWidth`, `WebkitTextStrokeColor` | Glyph-outline stroke: `"2px #ffffff"`, width in pixels/lengths, CSS color; width and color may animate |
| `fontVariationSettings` | CSS axis string such as `'"wght" 650.25, "wdth" 75'`; may animate per frame on `Text` and `Span` |
| `fontFeatureSettings`, `fontKerning` | Font-dependent typography; use a font containing the requested features |
| `fontVariant`, `fontVariantLigatures/Numeric/EastAsian/Caps/Position` | Static font-feature selection, for example `fontVariantNumeric: "tabular-nums"` |
| `fitText` | `fitText({ minFontSize, maxFontSize })`; see below |

Font size, colors, spacing and supported numeric styles may be animated, but text
metrics and line breaks can then change each frame. For a reveal that preserves
layout, animate opacity/translation instead of inserting/removing characters.
JSX indentation is not a reliable way to author spaces between rich runs: use
explicit `{" "}` or string expressions where spacing matters.
For outlined text, use `color: "transparent"` with `WebkitTextStrokeColor: "#ffffff"`
and `WebkitTextStrokeWidth: 1 + ctx.progress * 3`; the glyph outline stays attached
to the shaped text. This is available on `Text`, outside `Span`'s restricted styles.
For a padded caption/card, put padding on a containing `View` and use flex/grid
alignment or explicit positioning. Text in an inline formatting context does not
behave like a standalone block box with vertical margins.

The default sans face is variable Noto Sans: `fontWeight` maps directly to `wght`
(100–900), including fractional weights. It also has a `wdth` axis (62.5–100,
default 100). For example, `fontWeight: 100 + ctx.progress * 800` animates weight.
To animate width, use:

```tsx
fontVariationSettings: `"wght" 500, "wdth" ${62.5 + ctx.progress * 37.5}`
```

Explicit `"wght"` in `fontVariationSettings` overrides `fontWeight`.
The default CJK fallback is variable Noto Sans CJK SC, with its own `wght` axis
(100–900), so Chinese also follows continuous `fontWeight` changes. Axis availability
and ranges depend on the selected font; mono, symbol and emoji faces remain separate.
Measurement and rendering share the default font pack. Formula faces are separate.
See the [font inventory](../assets/fonts/README.md).
Additional fonts must be available at measurement and rendering time. Web hosts
consume the fonts in the supplied render package; do not assume an arbitrary
operating-system font is present in every renderer.

### Rich text and per-unit animation

`Span` is a direct `Text` child. Its styles are exactly `color`, `fontFamily`,
`fontSize`, `fontWeight`, `fontStyle`, `fontVariationSettings`, `letterSpacing`,
`opacity`; it does not accept general layout props or `className`. Multi-run text supports
`split/perUnit`, preserving each span’s style across animated units; see the
[rich text units fixture](../crates/valle-compiler/tests/fixtures/motion/composition/rich-text-units.motion.tsx).
Multi-run text-on-path remains unsupported. Inline `Image` is supported within Text, but
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
`rotate` (numeric angle), `color`, and `blur` (Gaussian sigma in CSS pixels,
0–128). `ctx.unit.*` is unavailable in ordinary styles.
When one `Text` contains styled `Span` runs, its units span the full authored
text: `ctx.unit.index`, `count`, `start`, and `end` do not restart at a `Span`.
Each run keeps its color, weight, and shaping; the unit style is applied to its
glyphs. Inline images are excluded from character counts and separate word units.

`rangeSelector({ start, end, offset?, softness?, shape? })` returns a 0–1 weight
for the current text unit, so use it inside `perUnit`. `start` and `end` are
normalized positions in 0–1; the selector samples each unit at `index / count`.
`offset` shifts both ends, while `softness` (0–1) feathers the boundaries. `shape`
is a static `"square"` (default), `"ramp"`, `"triangle"`, or `"smooth"`.
Numeric options may depend on the frame. An empty or reversed range selects nothing.

```tsx
<Text split="char" perUnit={{
  opacity: rangeSelector({ start: 0, end: ctx.progress, softness: 0.15, shape: "ramp" }),
  blur: 8 * (1 - rangeSelector({ start: 0, end: ctx.progress, softness: 0.15, shape: "ramp" })),
}}>SELECTOR</Text>
```

`Text path={path(...)}` lays out each shaped line along a parallel offset copy
of the path. It does not wrap by default. Use `whiteSpace: "pre-line"` or
`"pre-wrap"` for explicit newlines; `lineHeight` sets the distance between the
line paths. Make the path long enough for every line: glyphs beyond a line's
path are reported as unsupported. Styled `Span` children inside a path `Text`
are not supported. Path text does not expose individual formula/glyph fragments
as arbitrary selectable JSX nodes.

### Measuring and fitting

`measureText(text, { fontSize, fontFamily?, fontWeight?, fontVariationSettings?, letterSpacing?,
lineHeight?, maxWidth? })` runs during preparation and returns measured metrics
including `width` and `height`. `fontSize` is required; every option is the same
value, with the same meaning, as the identically named property in a JSX `style`
object, so measurement admits exactly what rendering admits. `measureText` does
**not** accept `className` or `style`: pass explicit typography, and write the
same numbers on the `Text` node you are sizing. Numbers are CSS pixels; `lineHeight`
is the unitless multiplier the CSS property is. String lengths such as `"1rem"` or
`"36px"` are not accepted — compute the pixel value in a `const` instead.

```tsx
const label = "全球业务网络 Overview";
const metrics = measureText(label, { fontFamily: "monospace", fontSize: 28, lineHeight: 1.25, maxWidth: 200 });
export default function Card() {
  return <Scene style={{ width: 640, height: 480 }}>
    <View style={{ width: metrics.width, height: metrics.height, backgroundColor: "#334455" }} />
    <Text style={{ width: 200, fontFamily: "monospace", fontSize: 28, lineHeight: 1.25 }}>{label}</Text>
  </Scene>;
}
```

`width` and `height` describe the layout border box of the measured text before
paint transforms, in CSS pixels. `maxWidth` supplies the available layout width for
wrapping; without it, measurement uses intrinsic max-content width. Text and options
must be prepare-time values. Measurement prepares an isolated `Text` node through the
same layout path, font stack and property admission as rendering, so supply the same
font bytes and canvas as rendering; an asset font is named with the same alias an
authored `fontFamily` uses (`fontFamily: "asset://brandFont"`). A surrounding JSX
parent's inherited styles are not part of the isolated request: pass the relevant
typography explicitly.
Font lists such as `"monospace, sans-serif"` and quoted names with fallbacks keep
their order and the same parsing as `Text`.

Measurements become artifact constants. Do not pass frame-varying text or options, and
do not use browser measurement APIs. Reprepare when fonts or the canvas change.
Low-level Rust callers also supply viewport DPR; metrics remain CSS pixels, with
device-pixel rounding determined by that fixed DPR.

`textOutline(text, { fontSize, fontFamily?, fontWeight?, fontVariationSettings?,
letterSpacing?, lineHeight?, maxWidth?, origin?, align? })` uses the same font stack and
layout as `measureText`, then returns the resolved glyph curves as static `PathData`.
`origin: point(x, y)` places the first line's baseline in scene coordinates; it defaults
to `(0, 0)`. `align` is `"left"` (default), `"center"`, or `"right"` relative to the
layout width. The result has `path`, `bounds`, and `glyphs`; each visible glyph has its
own `path`, baseline `x`/`y`, `advance`, source UTF-8 `cluster` offset, and zero-based
`word` and `line` indices. Whitespace without ink contributes to layout but has no
outline entry. Text and typography must be known during preparation, and the host
must supply the same font bytes used for rendering.

```tsx
const word = textOutline("VALLE", { fontSize: 150, fontWeight: 800, origin: point(60, 240) });
export default function DrawOn(ctx) {
  return <Scene style={{ width: 640, height: 360, backgroundColor: "#101010" }}>
    <Path d={word.path} fill="none" stroke="#ffffff" strokeWidth={3} trimEnd={ctx.progress} />
  </Scene>;
}
```

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
| `sector({ center, inner?, outer, start, end, cornerRadius? })` | Filled annular sector; frozen 16-command / 39-point table; inner and corner radius default 0; sweep at most TAU |
| `area(input,baseline)` | Close one open contour to a horizontal baseline, keeping Line/Quad/Cubic verbs |
| `areaBand(upper,lower)` | Close two aligned open contours (matching segments, strictly increasing X) |
| `offsetPath(input,distance)` | Geometric path offset |
| `resamplePath(input,count)` | Uniform arc-length polyline samples per contour; `count` is a static integer (2–2048 in frame expressions) |
| `reversePath(input)` | Reverse each contour's travel direction while preserving lines and Bézier geometry |
| `roundCorners(input,radius)` | Round line-only vertices with cubic circular fillets; geometric radius is non-negative and may animate |
| `zigzag(input,{size,ridges})` | Add alternating peaks to one open contour; size is non-negative and ridges is a static integer 1–1023 |
| `noiseDisplace(input,{seed,amount,frequency,phase?})` | Resample each contour to 256 points and apply seeded radial 2D noise; phase may animate |
| `puckerBloat(input,amount)` | Resample closed contours to 256 points; pull radial extrema toward their mean at negative amounts or amplify them at positive amounts; amount ∈ [−1,1] |
| `twist(input,angle)` | Resample contours to 256 points and rotate each point about its contour center by a radius-weighted angle in radians |
| `simplify(input,tolerance)` | Simplify contour shape by distance tolerance, then resample to 256 points so animated tolerance retains fixed topology |
| `strokeToPath(input,width)` | Expand a path to a filled outline with exact polyline corners and bounded miter joins; non-negative width, butt caps on open paths, a union of the valid stroke region on closed paths |
| `boolean(left,right,op)` | Path union/intersection/difference/xor; both inputs must satisfy geometry admission |
| `morphPath(from,to,progress)` | Interpolate compatible path topology |
| `morph(from,to,progress,options?)` | Prepare convex edge-direction, shared-kernel polar, checked arc-length, or fixed-boundary compatible-mesh correspondence. `options.method` may be `"auto"` (default), `"convex"`, `"polar"`, `"compatible"`, or `"arcLength"`; `anchors` fixes boundary points, `pairs` matches multiple contours, and `contactPolicy` is `"warn"` (default) or `"error"` |
| `morphSequence(paths,stops,progress,options?)` | Prepare 2–16 closed paths with convex, polar, checked arc-length, or compatible-mesh correspondence across keys; stops must be finite and strictly increasing. Accepts the same `method`, `anchors`, `pairs`, and `contactPolicy` options |
| `pointAt(path,progress)`, `tangentAt(path,progress)` | Sample path position/tangent |
| `pathLength(path)` | Path length |
| `pathTrajectory(paths,frame)` | Sample a prepared array of compatible path frames |

Use typed constructors where a Path/Point/Rect is required. Plain strings/objects
with similarly named fields are not automatically those types. For dynamic path
coordinates, use `pathTemplate` or geometry constructors instead of injecting
arbitrary commands into `d`. `deg(90)` gives a radian value for geometry APIs.

`resamplePath` includes both endpoints of an open contour. A closed contour gets
`count` distinct points and remains closed; a full-turn `arc` is treated as a
closed contour. `morph` computes its correspondence once from prepare-time path
inputs. Convex correspondence merges all edge directions and inserts zero-length
edges where a shape lacks a direction. Every intermediate contour is a convex
Minkowski combination, including when the endpoints have no shared kernel point.
`morphSequence` uses one edge-direction grid for all convex key shapes. For
non-convex star-shaped contours, the polar method uses one shared kernel point
and angular grid; the grid includes the original polygon corners so they survive
at the exact stop. For other single, simple, closed contours, `arcLength`
resamples each shape to 256 points, aligns cyclic starting points, and rejects
correspondences whose linear interpolation has a self-intersection, zero edge,
overlapping adjacent edge, or area collapse anywhere in the time interval.
The check converts the input floating-point coordinates to exact integer-grid
polynomials before comparing event times; it does not rely on checking a few
frames. `compatible` embeds each single simple closed polygon in a common
fixed convex triangle, refines both endpoint triangulations to the same mesh,
and interpolates positive barycentric neighbor weights. This keeps the
intermediate mesh valid when the numerical solve succeeds. A contour with fewer
flattened boundary vertices gains collinear points on its longest edges, so its
authored boundary stays geometrically unchanged. The current implementation
accepts at most 59 boundary vertices after this step and limits the solve to
128 interior mesh vertices. A sequence uses the largest key-shape vertex count
for every segment, so its output topology stays fixed across stops.
It may reject poorly conditioned geometry. It does not yet accept `anchors` or
`pairs`; those options remain available to the other methods. `auto` tries
convex, then polar, then checked arc length, and tries the compatible mesh if
the arc-length check fails and the inputs meet its limits. A failed explicit
`arcLength` check reports an event time and the involved vertex and edge.
`anchors` can fix boundary correspondence for `auto` or `arcLength`:
`[[point(ax, ay), point(bx, by)], ...]` for a pair, or one point per key shape in
each row for a sequence. Anchors must lie on their contours and keep the same
cyclic order; up to 32 rows are accepted. The prepared paths include every
anchor at the same vertex index across keys. `allowSelfIntersection: true`
explicitly bypasses the continuous-time check for an arc-length morph while
still requiring simple input contours. Crossings, folding spikes, zero edges,
and area collapse are rejected. A zero-width tangent contact emits a compiler
warning by default; set `contactPolicy: "error"` to reject it. Successful
browser compiles return these warnings alongside the artifact, while the CLI
prints them with source locations. For paths with multiple contours, `pairs`
explicitly matches their
zero-based contour indices: `[[0, 1], [1, 0]]` swaps two contours in a pair;
each row of a sequence has one index per key shape. Use `null` for a contour
absent at a key shape, for example `[[0, 0, 0], [null, 1, null]]` to open and
close a hole. Every existing contour must appear exactly once, with at most
eight rows. Missing contours become a point at that key shape. Outer contours
and holes keep their nesting role, and hole winding is normalized opposite the
outer contour. A missing hole starts inside its parent's filled region, away
from existing sibling holes; when several high-clearance points are available,
the point nearer the hole's visible key shape is preferred. By default,
preparation also rejects collisions between
different contours anywhere between stops. When `anchors` and `pairs` are both
provided, each anchor row must lie on exactly one matched contour at every key
shape; the row uses checked arc-length correspondence for that contour. A row
with a `null` contour cannot receive anchors. Ambiguous or mismatched contour
membership is rejected. Without `pairs`, multiple contours are matched within
their nesting level by a deterministic minimum-cost comparison of their bounds;
contours with no match use `null`. Anchors constrain this matching, including
the containing contours of anchored holes. The same continuous-time checks
apply to inferred pairs; if they reject a match, specify `pairs` to choose the
correspondence.
Unsupported correspondence options are rejected.
`morphPath` still accepts authored compatible topology without this check.

Compatible-mesh interpolation guarantees a simple boundary, not local rigidity.
Intermediate outlines receive an orientation-preserving similarity correction: their
centroid follows the authored endpoints, and their extent stays within the interpolated
endpoint bounds. This prevents global drift and unwanted growth; local features may
still deform. Authored endpoint geometry is unchanged.

`roundCorners` retains a fixed Line/Cubic command pattern while its radius changes.
Tangent distance is derived from the corner angle and radius; each adjacent edge
limits that distance to half its length. Cubics approximate the circular arc.
`zigzag` fixes its point count from `ridges`; `noiseDisplace` uses fixed resampling
and recenters each contour after displacement. Their frame parameters change
coordinates without changing command topology.
`puckerBloat`, `twist`, and `simplify` also use fixed-size output
contours. `strokeToPath` unions stroked segments, preserving straight-edge corners and
surviving holes. Curved runs are simplified when needed to keep the output within the
2,048-point frame budget; too many essential corners produce a budget error.
Its contour topology can change as width closes a hole or joins overlapping regions;
do not assume two such outputs have compatible topology for `morphPath`. `simplify` reduces visible corners but retains the output point count;
it is a shape operation rather than a vertex-count optimization.

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
Frame-time gradient colors and geometry may be assigned to a local `const`, aliased, and
passed to a component as a paint prop before use in `fill`, `stroke`, or `paint`.

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
- one direct `MaskSource` child, in any child position.

`mode` is `alpha` (default) or `luminance`, for all source types. Luminance is measured
in linear Rec.2020 and includes the source's alpha. `invert={true}` reverses coverage.
A `MaskSource` is rendered into its own subgraph, is not independently visible,
and defaults to an absolute box filling the mask, so it does not displace content.

`mixBlendMode: "plus-lighter"` adds premultiplied colors in linear light; two opaque
`#800000` layers produce approximately `#af0000`. Other supported CSS blend modes
use encoded sRGB by default. Set `mixBlendSpace: "linear"` to calculate them on
linear sRGB channels: screening two opaque `#800000` layers produces about
`#a70000`, compared with `#c00000` in the default space. The setting applies to
the node's composed subtree, is not inherited, and can switch per frame using
a finite choice such as `ctx.seconds < 1 ? "linear" : "srgb"`.
`normal` and `plus-lighter` always use linear working-space compositing, regardless
of this setting. Alpha coverage is unchanged. Unsupported operators such as
`plus-darker` and unknown spaces are rejected during admission.

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

### Subtree transitions

`<Transition kind="circleOpen" progress={p}>` requires exactly two direct child
subtrees: outgoing first, incoming second. `kind` is static and supports the same
13 kernels as Timeline: `fade`, `wipeLeft`, `wipeRight`, `circleOpen`, `simpleZoom`,
`crossWarp`, `linearBlur`, `directionalWarp`, `dreamyZoom`, `ripple`, `flyEye`,
`multiplyBlend`, and `perlin`. Unknown kinds and unsupported attributes are errors.

Optional `params={{...}}` accepts the [same names, defaults and ranges as Timeline](timeline.md#transitions).
The object keys must be static; each value may be a number or admitted numeric
animation expression. For example, `params={{centerX:0.2+0.6*ctx.progress,softness:4}}`
animates a circle reveal's center while holding its edge softness at four local
pixels. Values are checked after evaluation on every frame. Unknown names,
non-numeric or non-finite values, out-of-range results, and a zero direction for
`directionalWarp` are errors. Missing fields use the kind's defaults.

Both children render independently against transparency, in the Transition's local
border box. By default each is absolutely positioned at `(0,0)` and fills that box;
explicit child sizes and positions override these defaults. Give the Transition a
positive size. Child `z-index` cannot reverse the two inputs. Empty or hidden inputs
are transparent. A zero-size or hidden Transition paints nothing.

`progress` must be a finite number in `[0,1]`. Endpoints 0 and 1 reproduce the
corresponding subtree, clipped to the local canvas; 0.5 uses the kernel's midpoint.
Masks, filters and nested transitions work inside either subtree. Owner styles
such as opacity, clipping and filters apply to the combined result. Current-backdrop
reads inside an input start with that input's own transparent canvas.

```tsx
export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function Reveal(ctx) {
  const p = clamp(ctx.seconds, 0, 1);
  return <Transition kind="circleOpen" progress={p} style={{width:640,height:360}}>
    <View style={{backgroundColor:"#2140ff"}} />
    <View style={{backgroundColor:"#ff4a24"}} />
  </Transition>;
}
```

`ctx.progress` is `localFrame / durationFrames`: for a 60-frame composition, frame
30 is exactly 0.5 and frame 59 is 59/60. Use an explicit time expression when the
last rendered frame must reach 1. The [E06 fixture](../crates/valle-compiler/tests/fixtures/motion/composition/motion-transition.motion.tsx)
uses `ctx.progress` as authored in the refactor acceptance scene.

### Displacement and motion blur

`style.displacement: displacement(seed, frequencyPoint, scale, options?)` distorts
a node and its descendants. `style.backdropDisplacement` uses the same constructor
to distort the backdrop. Seed is a static integer in `[0, 2^32−1]`; options are
`octaves` (1–4) and `mode` (`fractal` or `turbulence`). Frequency is a point,
scale a number; both may use admitted animation expressions.

`style.motionBlur: "auto"` derives directional blur from the element's final
screen position. It samples the scene half a frame before and after the requested
output time, so layout changes, ancestor transforms, and camera movement also
contribute. The built-in shutter angle is 180°. At a temporal cut or when the
element is stationary, no blur filter is emitted. The result is determined by
the requested time, including for out-of-order frame requests.

`style.motionBlur: motionBlur(velocityPoint, shutterAngle?)` supplies explicit
velocity in pixels/frame. Its shutter angle defaults to 180°. Both forms use a
spatial directional blur based on the element's anchor velocity. Rotation and
scaling can give different velocities to different pixels, so they require
shutter sampling for a faithful result.

Use `springVelocity` for an exact spring derivative. For an arbitrary pure position
helper `position(t)`, a centered one-frame displacement is
`position(t + 0.5 / fps) - position(t - 0.5 / fps)`, already in px/frame. This is an
approximation of instantaneous velocity and explicitly samples the same authored
trajectory on both sides of the target time. It never reads the last displayed
frame. With explicit velocity, a step cut still needs an authored blur decision;
`"auto"` suppresses blur when adjacent screen-space slopes disagree sharply.

Effects on a group include its descendants. Keep the affected bounds small and
inspect clipping near edges. See
[advanced node effects](../crates/valle-compiler/tests/fixtures/motion/effects/advanced-node-effects.motion.tsx)
for explicit velocity and displacement examples, and the
[automatic motion blur scene](../crates/valle-compiler/tests/fixtures/motion/composition/auto-motion-blur.motion.tsx)
for automatic blur.
The additional layer styles `paperGrain` and `contactShadow` take finite numeric
intensities clamped to 0–1. They are Motion layer effects, not browser CSS properties.

### Shutter sampling

`<Shutter samples={8} angle={180}>...</Shutter>` evaluates its children at eight
output-time samples across a 180° exposure (half a frame) and averages their
premultiplied pixels in linear light. This reproduces rotation and scaling blur
that one anchor velocity cannot describe. Every requested frame computes its own
subframes, so seeking and out-of-order rendering give the same result. Samples
outside the composition hold its first or last frame.

`samples` is a static integer from 1 to 32; `angle` is a static integer from 0°
to 360°. A frame may request at most 128 samples across Shutter and Echo nodes. The
Shutter wrapper fills its parent and takes no authored styles, classes, or
visibility. Put those properties on a surrounding `View`. Nesting Shutter or
Echo within either temporal sampling effect is currently rejected.

### Echo trails

`<Echo count={4} interval={3} decay={0.5}>...</Echo>` samples its children at
3, 6, 9, and 12 output frames before the requested frame. It paints the oldest
sample first with opacity `decay^4`, then successively newer samples with
`decay^3` through `decay`, and finally paints the current children at full
opacity. Each sample reevaluates Motion expressions, layout, and media time, so
the trail follows the scene rather than a fixed spatial offset. Samples before
the composition starts hold the first frame; seeking does not depend on earlier
render requests.

`count` is a static integer from 1 to 32, `interval` is a positive integer
number of output frames, and `decay` is a finite static number from 0 to 1.
Shutter and Echo together may request at most 128 temporal samples in one
frame. The Echo wrapper fills its parent and takes no authored styles, classes,
or visibility; use a surrounding `View` for those properties. Nesting Shutter
or Echo within either temporal sampling effect is currently rejected.

### Time scopes

`<TimeScope offset={0.5} speed={2}>...</TimeScope>` maps the time seen by
its descendants to `(parentSeconds - offset) * speed`. It applies to
`ctx.seconds`, the derived `ctx.localFrame` and `ctx.progress`, and Video source
time. Nested scopes compose from outside in; siblings outside the scope keep
the parent clock. `ctx.fps` and `ctx.durationFrames` retain the composition
contract. Speed zero freezes the subtree; a negative speed reverses its local
clock. Expressions can see a local time outside the composition's output
interval.

`offset` and `speed` must be finite static numbers. TimeScope contributes no
layout box or paint of its own and takes no authored styles, classes, or
visibility; use a surrounding `View` for those properties.

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
Progress is not clamped: springs and custom easing can overshoot or undershoot.
Translation and scale extend linearly beyond the endpoints, with each scale axis
floored at zero to prevent a collapsed rectangle from reflecting its subtree.
If a transition should stop at its endpoints, pass `clamp(progress, 0, 1)` explicitly.

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
`ctx.seconds`/`ctx.progress`; discrete `localFrame` is restricted
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
| `scaleSequential({ domain:[min,max], colors })` | `.map(value)` continuous mix through at least two CSS colors; out-of-range clamps |
| `scaleQuantize({ domain:[min,max], colors })` | `.map(value)` equal bins, one or more colors; edges belong to the right bin, max uses the last color |
| `stack(values,{ offset }?)` | Stacked intervals; default offset `zero` |
| `pie(values,{ startAngle?, endAngle?, padAngle? })` | Input-order slices `{ index, value, startAngle, endAngle, midAngle, fraction }`; zeros keep a zero sweep; no implicit sort |
| `curve(points,{ type: "monotoneX" })` | Prepare-time monotone cubic through strictly increasing-X points; one cubic per pair; gaps fail |
| `geoProject({ points, projection?, width, height, padding? })` | Batch projection/fitting of geographic coordinates |
| `geoPath({ polygons, projection?, width, height, padding?, clip? })` | Projected polygon paths |
| `placeLabels(candidates,{ bounds, padding? })` | Deterministic placement of label candidates |
| `graphLayout({ sizes, edges, nodeGap?, rankGap? })` | Graph ranks, rows, back edges and node centers |
| `seededRandom(seed,count,range)` | Prepared pseudorandom values with explicit seed |

`pie` keeps input order and zero-value slots; do not sort slices. Grow a wedge by
interpolating `sector` start/end/inner/outer/cornerRadius, then constructing the
path; do not morph cubic controls of two arcs. `sector` always emits the same
commands so a hole stays unfilled under the default nonzero fill rule. `curve`
fits once at prepare time; reveal with `trimEnd` or `morphPath` between two
prepared curves of the same point count. Grouped bars use two nested `scaleBand`
calls; polar positions use `point` plus `sin`/`cos`.

Inner and outer sector corners are limited independently. Corners shrink with the
remaining angular gap as a sector closes into a full circle. `areaBand` requires
matching X coordinates at every corresponding boundary node; it does not align
data automatically. `pie` rejects a non-finite value sum and normalizes values
before multiplying by the angular span.

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
including donut share, smooth area, stacked area, and heatmap/gauge films,
[map fixture](../crates/valle-compiler/tests/fixtures/motion/maps/map-atlas.motion.tsx)
and [data dashboard](../crates/valle-compiler/tests/fixtures/motion/modules/data-dashboard/dashboard.motion.tsx).
Valle exposes these lower-level builders; it does not currently provide a general
`Chart`, `FunctionPlot` or arbitrary HTML widget primitive.

### Automatic instances for prepared maps

Motion compiles a prepared array's `.map()` to one shared template and a columnar instance table
when every item returns the same supported shape with a direct `key={item.id}` field.
For a batch-drawn `View`, the leaf needs
`className="absolute"`, and a literal style containing `left`, `top`, `width`, `height`, and
`backgroundColor`. The optional `opacity` expression and `rotate(...deg)` transform may depend on
time, item fields, the callback index, and `ctx.instance.index` / `ctx.instance.count`. These views
draw as one rectangle batch. A full solid `Circle` with no stroke also shares one template;
its center, radius, fill and optional opacity can vary by row. Each circle still draws its exact
authored arc. A `Path` can share one static nonempty geometry and solid fill with pixel `translate`,
optional numeric `opacity`, and optional `rotate` angle, numeric/point `scale`, or a single
`skewX(...deg)` in `style.transform` when `transformOrigin: point(0, 0)` is explicit.
Translation, fill, opacity, rotation, scale and horizontal skew can vary by row and frame;
it draws as one path batch. The batch accepts fill-only, stroke-only, and separately colored
solid fill/stroke. Positive `strokeWidth` and `strokeDashoffset` may vary by row and frame;
cap, join, miter limit and dash pattern are shared template values. Omitting stroke width uses
1; a stroked Path still requires a positive width. Other unsupported paints and transform
lists use ordinary per-row paths. See the
[instance grid](../crates/valle-compiler/tests/fixtures/motion/composition/grid-instances.motion.tsx),
[Circle](../crates/valle-compiler/tests/fixtures/motion/composition/circle-instances.motion.tsx), and
[Path](../crates/valle-compiler/tests/fixtures/motion/composition/path-instances.motion.tsx) fixtures.
An in-flow `View` or `Group`, or a plain leaf `Text`, uses the layout instance path: one compiled
expression tree, but separate layout boxes and paint for every row and descendant. A `View` or
`Group` root may contain a fixed nested tree of `View`/`Group` nodes and plain leaf `Text` nodes.
Supported inline layout, color and text styles may vary by row; `visible={...}` hides paint while
preserving its layout box. A root or descendant can use a static or finite conditional `className`,
including a class-only layout with no inline style. Row data may supply text content, class
conditions and descendant styles. Descendant keys follow the row key and JSX child path.
`bounds()` and `anchor()` can read row keys and descendant `View`/`Group` layout boxes.
The map may also return the same authored component for every item, such as
`items.map(item => <Card key={item.id} item={item} />)`, when it returns one of these fixed
in-flow shapes. The component may read scalar or typed item fields through `item.field`,
`props.item.field`, or local destructuring, and may forward the item to another fixed component.
Only fields actually read by the template enter the instance table. Component and descendant
keys retain the same paths they have when the map expands normally. Unread item metadata, including
nested objects or fields whose value types vary between rows, does not prevent instancing.
See the [flow list](../crates/valle-compiler/tests/fixtures/motion/composition/flow-instances.motion.tsx)
and [card list](../crates/valle-compiler/tests/fixtures/motion/composition/repeated-cards.motion.tsx),
including its [component form](../crates/valle-compiler/tests/fixtures/motion/composition/repeated-card-component.motion.tsx),
and the [Text root list](../crates/valle-compiler/tests/fixtures/motion/composition/text-root-instances.motion.tsx).
Other maps retain per-item JSX lowering. Maps with at least 64 items emit an
`instance-fallback` warning with the source position and reason; smaller maps do not
produce this optimization warning. Path trimming currently uses that path.
The instance table stores numbers, points, colors, rectangles, booleans and strings in
homogeneous columns; template expressions reconstruct only the values needed for each row.
When an `Array.from` row field is a numeric function of its index using arithmetic,
`Math.floor`, or `Math.sqrt`, the compiler may store the function instead of the numeric
array. It checks the function against every prepared value first; fields that do not match
remain ordinary columns. The source array is still evaluated during preparation.
Keys of the form `prefix + rowIndex + suffix` are also checked against every authored key and
stored as one indexed key rule; other keys remain explicit. Both forms resolve to the same
per-row identity for diagnostics.

`valle motion check --json` reports `templates`, `instanceRows`, the total template expression
count, and `timings.templateCompile` / `timings.instanceData` in milliseconds. The timing fields
measure template JSX lowering and instance-table construction separately.
An instance template may use `visible`. Batch-drawn templates can select a visible prefix or
suffix with a logarithmic search when visibility compares composition time or local frame against
the instance row index. Other batch visibility expressions evaluate per row. Layout instances
always retain each row's box, including hidden rows. A batch-drawn row whose geometry is read by
`bounds()` or `anchor()` stays an ordinary scene node.

### GeometryBatch and particles

Use `GeometryBatch` when many circles, rectangles, copies of one prepared path, or tiles from one image share one coordinate space and
do not need separate layout/children. Flow cards with supported fixed `View`/`Group`/`Text`
subtrees can use layout instances; maps with other content use ordinary `.map()` nodes.

`geometry` is `circle`, `rect`, a prepared `PathData` value such as `path("M...")`,
or `atlasRegion("asset://sprites", rect(0, 0, 0.5, 1))`. The image source must be
declared as an image asset control. The region is a positive normalized source rectangle
inside the image; use source pixel boundaries for clean tile edges. Atlas rows use
`positions` as their top-left corners and `sizes` as destination widths and heights;
rotation is around each destination rectangle's center. `fills` tint each tile, and
`opacities` apply per row. Image rows do not accept a stroke.
See the [two-tile atlas example](../crates/valle-compiler/tests/fixtures/motion/composition/atlas-geometry-batch.motion.tsx).
Path instances use their `positions` as the local path origin and their `sizes` as
per-axis scale factors. Rotation is in degrees around that origin. `positions` is a prepared point array or a
supported field. `sizes` and `fills` are required; scalar values broadcast across
instances, while arrays follow the batch contract. `opacities` and `rotations`
are optional; rectangle rotations are degrees around each rectangle's center.
`skewXs` adds a horizontal shear in degrees before rotation (restricted to less than 89° in magnitude),
also centered on each rectangle. `strokeWidths` adds a same-color stroke in local shape units:
rectangles use a unit box, circles a unit radius, and paths their authored coordinates.
Both values default to zero and can be a scalar or a per-instance array.
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
          forces: [curlNoise({ seed: 3, scale: 0.012, strength: 220 }), drag(0.8)],
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
typed gravity and optional loop. With particle fields, two-element sizes, fills,
opacities, rotations, skewXs and strokeWidths describe the start/end values over particle life. It is deterministic
at an arbitrary frame; it does not require rendering every previous frame.
For looping fields, `count × birth.interval` must be at least `lifetime`.
Optional `forces` is a prepare-time array of up to eight `curlNoise({seed,scale,strength})`
or `drag(coefficient)` values. Curl noise uses a positive spatial scale and a signed
acceleration strength; drag uses a nonnegative coefficient per second. Force trajectories
are baked at 120 samples per second when the scene is prepared, then interpolated for
arbitrary frame seeks. The table is limited to four million particle samples per batch.
Setting a force's strength or coefficient to zero has the same result as removing it.

For fixed batches, `field({ from, to, progress, stagger? })` can bind positions,
sizes, fills, opacities, rotations, skewXs or strokeWidths. From/to are static compatible arrays (or admitted
broadcast values for side fields); progress is numeric and can animate. `stagger`
accepts either a prepare-time nonnegative step, giving instance `i` delay `i × step`,
or a prepare-time array with one normalized delay in `[0,1)` per instance. The array
can be non-monotonic, for example `[0.4,0.2,0,0.2,0.4]` starts an animation at the
center and propagates outward. The remaining progress interval is remapped so every
instance reaches `to` at progress 1. For a step, the final instance's delay must
remain below 1. Keep array lengths and geometry types consistent. See
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
FOV defaults to 38° and is restricted to 10–120°; near defaults to 0.1 and far to 1000.
Optional `depthOfField: { focusDistance, maxBlurRadius }` accepts frame expressions
for both values. `focusDistance` is the camera-space distance of the sharp plane
(0.01–100000 world units), and `maxBlurRadius` caps the defocus circle at 0–16
output pixels. Omission or a zero radius disables the effect. The depth-aware
post-process softens geometry before and after the focus plane and spreads blurred
foreground silhouettes over more distant pixels. It changes the Scene3D color
plane; depth, anchors and picking retain the original geometric values.

```tsx
<Scene3D
  camera={{ position: [0, 0, 3], target: [0, 0, 0],
    depthOfField: { focusDistance: 3 + ctx.seconds * 0.5, maxBlurRadius: 12 } }}
  style={{ width: 640, height: 360 }}>
  <Mesh key="model" src="asset://model" />
</Scene3D>
```

Its direct children are explicit leaves, not arbitrary React/Three.js content:

| Child | Attributes |
| --- | --- |
| `Mesh` | Static `key`, exactly one of `src="asset://model"` or `geometry={extrude(...)}`, `geometry={lathe(...)}`, `geometry={tube(...)}`; `material` and indexed `materials` overrides; animated `position`, `rotation`, `scale` triples and `translateX/Y/Z`, `rotateX/Y/Z`, `scaleX/Y/Z`; model-node `nodes` bindings and imported GLB `animation` |
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

Prepared paths can build meshes without a model asset:

| Function | Prepared path and options |
| --- | --- |
| `extrude(path, { depth, bevel? })` | Closed, nonintersecting contours; nested contours form holes. `depth` is positive and `bevel` ranges from zero to half the depth. Front and back caps, walls, and bevel receive normals and UVs. |
| `lathe(profile, { segments? })` | One simple open or closed profile. Profile X is the radius from the world Y axis. Open profiles may reach zero only at endpoints and, together with their axis caps, must bound a simple region with nonzero Y extent; returning lips are allowed. Closed profiles stay at positive radii and receive no caps (for example, a torus). `segments` defaults to 64 and ranges from 3 to 256. Nonzero radius endpoints receive disk caps. |
| `tube(path, { radius, sides? })` | One planar centerline. `radius` is positive; `sides` defaults to 16 and ranges from 3 to 128. An open path receives end caps; a closed path joins its last ring to its first without caps. Zero-length edges and reversing corners are rejected. |

Path Y uses the 2D screen-down convention and is flipped into the 3D world-up axis.
The tube centerline lies in the world XY plane; its circular cross section extends
through Z. All geometry is static; Mesh transforms and materials may animate.
Generated vertices use indexed storage. Angle-weighted normals smooth adjacent
faces within 60 degrees while retaining hard edges and UV seams.
The generated model has one mesh node with ID 0 and participates in the same
lighting, shadows, depth, picking and geometry budgets as a GLB model. It has no
source material indices, so use the whole-Mesh `material` override.

```tsx
const glyph = textOutline("V", { fontSize: 220, fontWeight: 800, align: "center" });
<Mesh key="letter" geometry={extrude(glyph.path, { depth: 60, bevel: 6 })}
  material={{ type: "pbr", color: "#8b7bff", metallic: 0, roughness: 0.4 }}
  position={[0, -80, 0]}
  rotateY={ctx.seconds * 40} />

<Mesh key="vase" geometry={lathe(path("M 0.4 1 L 0.7 0 L 0.3 -1"), { segments: 64 })}
  material={{ type: "lambert", color: "#77bbee" }} />
<Mesh key="rail" geometry={tube(path("M -1 0 L 0 0.4 L 1 0"), { radius: 0.12, sides: 16 })}
  material={{ type: "lambert", color: "#ffbb66" }} />
```

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
support `opaque`, `mask` and `blend`. Opaque ignores base alpha. Mask multiplies factor
alpha by texture alpha, discards values below the cutoff (default 0.5), and writes no
color/depth/picking data for discarded fragments. Retained fragments are opaque.
Blend uses factor alpha times texture alpha for weighted blended order-independent
transparency. Opaque and retained mask fragments establish depth first; transparent
fragments behind them are discarded. Visible transparent fragments accumulate a
depth-weighted color and revealage before compositing, so their submission order does
not change the result. This is an approximation for overlapping transparent surfaces.
Picking and the final depth plane use the nearest contributing transparent fragment.
Double-sided materials render both faces with reversed back-face normals. Normal and
occlusion maps share the same semantics for lit materials; occlusion affects indirect
illumination.

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
animation player is involved.

An imported glTF node animation can be sampled at an explicit time in seconds:

```tsx
<Mesh key="model" src="asset://model"
  animation={{ clip: 0, time: ctx.seconds }} />
```

`clip` is a static zero-based index in the GLB (at most 32 clips); `time` may be a
frame expression from 0 to 1e9 seconds. Times before the first key or after the last
key hold the nearest pose. The renderer never advances a clock. Translation, rotation,
scale and morph weights support glTF `LINEAR`, `STEP` and `CUBICSPLINE` interpolation;
linear rotation uses quaternion spherical interpolation. Channels use one shared
time even when their key ranges differ. Each frame starts from the source node TRS,
applies the clip, then applies explicit `nodes` replacements, so a replacement wins
for its complete local transform. Morph weights on a node override `mesh.weights`;
missing weights start at zero. A weight animation targets the instanced node, so
another node using the same mesh keeps its own weights. Explicit `nodes` entries
replace transforms only.

Skinned GLBs use `JOINTS_0` and `WEIGHTS_0` with up to four influences per vertex.
The importer reads the skin's joint list and optional inverse bind matrices (identity
when absent). Each frame samples joint transforms from the same explicit clip time,
applies morph targets before skinning, and blends joint transforms per vertex.
As required by glTF, the transform on the node holding the skinned mesh is ignored;
ancestor transforms on the joint hierarchy still apply. Explicit `nodes` replacements
can pose joints without an imported animation. A skin admits up to 128 joints and
16 MiB of decoded vertex influences.

Static GLB scenes admit multiple roots, parent/child nodes and shared mesh instances.
Node TRS and column-major affine matrices remain separate from source geometry;
matrices must decompose into non-singular TRS. Negative scales are supported, with
inverse-transpose normals and mirrored winding. Node identifiers are their original
glTF indices, independent of display names and traversal order. The importer retains
up to 256 nodes/meshes and a maximum hierarchy depth of 32. Picking returns the
scene object's semantic address plus `nodeId`, the original glTF node index.
The raster keeps separate object/node planes, with 20 bytes per pixel for half-float
color, 32-bit depth and both 32-bit identifiers; buffer reuse resets every plane.

Triangle primitives accept packed or interleaved FLOAT positions/normals, FLOAT or
normalized U8/U16 UVs, U8/U16/U32 indices and non-indexed geometry. Missing normals
produce flat face normals with split corners. UVs may be absent when no material
texture needs them; binding a texture without UVs reports an error. Buffer-view targets
are optional, and present targets must match their use. Morph targets may displace
positions and normals; absent target attributes leave the source value unchanged.
The renderer recomputes flat normals after deformation when the source has no normals.
Tangent accessors are validated but source and morphed tangents are not consumed by the
current rasterizer. A mesh admits up to eight targets and 64 MiB of decoded morph data;
weight magnitudes are limited to 16. External URIs, sparse accessors,
additional joint/weight sets and extensions remain outside the admitted profile.

Environment lighting uses an explicitly bound `asset({kind: "environment"})` resource:

```tsx
<Scene3D pbr={{
  environment: { src: "asset://sky", intensity: 1, rotation: ctx.seconds * 30, background: false },
  toneMapping: "aces", exposure: 1.1, shadows: true,
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
`shadows` is a static boolean, false by default. When enabled, each directional
light gets a 1024×1024 orthographic depth map rebuilt from the complete current
frame, including casters outside the camera view. Opaque and retained mask fragments
cast shadows; blended fragments do not. A 3×3 percentage-closer filter attenuates
the direct Lambert/PBR light term, leaving ambient, environment, emissive and Unlit
color unchanged. This is a single map per light, so very large scenes can show
limited shadow detail.


The loader admits a 64 MiB GLB, 262,144 vertices and 131,072 triangles. A Scene3D
layer can be 3840×2160 within the 8,388,608-pixel budget. A layer
admits at most 8 textures, 24 Mi pixels and 128 MiB decoded mip/environment storage.
Geometry storage is shared between nodes, while vertex and triangle work is charged
for every rendered instance. A prepared output frame additionally allows at most
16,777,216 pixels across distinct Scene3D raster requests (20 bytes per framebuffer
pixel), including temporal samples. Frozen model bytes plus vertex/index buffers,
frozen environments and external texture mip estimates share a 256 MiB frame
asset budget; identical content is charged once. Embedded GLB texture, morph and
skin storage still obey their per-layer admission limits. These limits apply per
worker, so multiple Native workers multiply the working set. Over-budget inputs
fail before drawing.

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
export const controls = {
  assets: {
    effect: asset({ kind: "shader", required: true }),
    noise: asset({ kind: "image", required: true }),
  },
};
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
| `StyleUnknownProperty` | Check the property name; arbitrary properties use canonical kebab-case CSS names |
| `StyleUnsupportedProperty` / `StyleUnsupportedValue` | Follow the property's specific reason and supported alternative |
| `StyleInvalidValue` | Correct the CSS value, units, numeric range or trailing tokens |
| `SandboxForbidden` | Move I/O/time/locale-dependent work outside Motion; bind its results as data |
| `StaticEvalFailed` / `BuiltinRejected` | Check argument shape, prepare-time requirements, ranges and finite values |
| Missing image/font/video | Declare the matching asset control and pass `--asset name=path` |
| Text looks different after export | Match font resources, canvas, duration and FPS; inspect wrapping/fallback |
| Border is missing | Set a positive border width and visible color; check for an explicit `border-none` or `borderStyle: "none"` override |
| Open path fills an unexpected shape | Set `fill="none"` for stroke-only intent |
| A blur is rejected at certain frames | Clamp its computed radius to non-negative values |
| Post-layout dependency error | Keep bounds/anchors out of size, ordinary layout and text content |
| PNG and MP4 show different animation states | Match all render inputs and compare the same zero-based frame |
| Slow export | Inspect bounded layers/blur/overdraw first; select backend/workers and encoding separately |

CSS values and expanded utilities share property diagnostics. Machine-readable compiler
diagnostics retain `style.kind`, `style.property`, `style.value` and `style.reason`, plus
the source span, node path and original `utility` when available. The style value is the
CSS value after utility decoding; the original utility preserves authored spelling.
All candidates in a finite class choice are validated, including currently inactive
branches. Type errors and unsupported expression structure can fail before CSS validation.

Compiler diagnostics name the replacement for each permanent rejection. Those are
property policies, not a guarantee that every CSS value or renderer combination works;
the limits and tested scenes remain authoritative.

Unsupported authoring patterns include React hooks/effects, DOM access, browser
events, local and remote stylesheets, selectors and media queries, CSS keyframe animations,
arbitrary npm modules, runtime network/filesystem access, unseeded randomness,
dynamic resource identities and frame-varying tree sizes.

Use `Clip`/`Mask` instead of CSS `clip-path`/`mask-image`; use `motionPath` instead
of CSS `offset-path`; use the supported ordered transform list for 2D skew/matrix.
An upstream CSS parser may recognize an otherwise unsupported declaration. Always
check the rendered frame and any prepare/render diagnostics; `motion check` cannot
prove every future frame's numeric validity or every backend's visual behavior.

Current compiler budgets include 10,000 items per expanded static JSX map, 50,000 expanded
list items across compilation, 2,000 helper expansions, 4,096 items for expression
map unrolling and expression nesting depth 256. Themes have bounded depth/size;
sequences allow at most 256 stages. These are rejection ceilings, not performance
targets. Supported `View` and `Path` instance templates can use up to 100,000 rows;
flow subtrees have a 32-level, 1,024-node template limit and one million projected
row nodes. Exact-arc `Circle` templates remain capped at 10,000. Prefer explicit batches for other
large repeated geometry.

For export controls, `--backend raster` selects CPU composition, `metal` requires
Metal and `auto` chooses an available backend. `--workers` accepts 1–8; defaults
are at most two for Raster and one for Metal. `--hardware-encode` is independent
of the compositor and fails if the requested hardware encoder is unavailable.
`--ffmpeg-log-level info` adds encoder diagnostics on stderr; `--events` adds
machine-readable progress on stdout. See [CLI output](cli.md#output-contract-for-scripts-and-agents).

## Examples and verification

Complete snippets above are intended to be saved as the named `.motion.tsx` files.
Unless another command is shown, inspect frame 45 with a fresh PNG output path, using
the `composition` the snippet declares. Try early and late frames for animated scenes.
`poster` needs an image, `bars` its JSON, and `Dissolve` its package/image. Source-only
snippets or constructor signatures are not standalone scenes.

This reference follows the compiler, layout adapter and native renderer. The
[authoring regression tests](../crates/valle-compiler/tests/motion_authoring_regressions.rs)
cover list identity and unknown style fields. The
[CLI regression tests](../crates/valle-cli/tests/timeline_motion_regressions.rs)
exercise Glass, ShaderLayer and per-instance image bindings.
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
| Donut share (pie / sector) | [donut-share](../crates/valle-compiler/tests/fixtures/motion/charts/donut-share.motion.tsx) |
| Smooth area (monotone curve) | [smooth-area](../crates/valle-compiler/tests/fixtures/motion/charts/smooth-area.motion.tsx) |
| Stacked area (`areaBand`) | [stacked-area](../crates/valle-compiler/tests/fixtures/motion/charts/stacked-area.motion.tsx) |
| Heatmap and gauge | [heatmap-gauge](../crates/valle-compiler/tests/fixtures/motion/charts/heatmap-gauge.motion.tsx) |

For maintainers, changes to the public surface should update this guide together
with a meaningful example/test. Source owners:
[JSX primitives](../crates/valle-compiler/src/motion/jsx.rs),
[controls](../crates/valle-compiler/src/motion/controls.rs),
[expressions](../crates/valle-compiler/src/motion/expr.rs),
[styles](../crates/valle-compiler/src/motion/style.rs),
[Tailwind admission](../crates/valle-motion/src/tailwind.rs),
[layout admission](../crates/valle-motion/src/layout/scene.rs),
[drawing emission](../crates/valle-motion/src/emit.rs).

## Static layout reuse

The prepared Motion engine can retain one immutable layout geometry snapshot per
active scene/font configuration. It admits only fixed topology, fixed text and
static layout inputs, with dynamic `translate`, `scale`, `rotate` and `opacity`.
Props and stable viewport/FPS/duration expressions may participate in layout;
changing these inputs invalidates the snapshot. Fonts are frozen at preparation,
and a replacement artifact or font set creates a new cache.

Unsupported nodes, dynamic layout/text, post-layout
dependencies, 3D and complex effects use the full layout path. A hit skips tree
construction and layout solving, while evaluating the requested frame and
rebuilding its current paint state and DrawProgram. It never reuses a previous
frame's rendered result. Cache hits, misses, replacement and request order have
the same normalized DrawProgram bytes. This reduces preparation work; drawing
and presentation costs remain separate.

## Studio property curves

The Motion inspector's **Property curves** panel samples explicit `opacity`,
`translate`, `scale` and `rotate` bindings for the selected child. Choose an
source frame window; switch between frames and seconds.
Click a plotted source frame to seek the existing preview and use its source
location to inspect the authoring code.

Sampling runs in a separate worker through the existing Rust/Wasm evaluator,
using the active artifact, exact FPS, props and viewport.
The request is bounded to one node, four properties and at most 240 actual source
frames. Closing/hiding the panel or changing inputs cancels obsolete work.
Playback only updates the playhead; it does not resample the curves.

Dots are sampled frames; connecting lines do not prove continuous motion. Speeds
are adjacent-frame central differences in px/s, degrees/s or property units/s.
Endpoints and recognized branch/segment/discrete boundaries omit uncertain speed.
The first/last observed change is only as precise as the sampled window. This is
a local-property inspector: inherited styles, parent transforms, relative length
units, per-unit text and layout-dependent expressions are not presented as
resolved world-space velocity. Unsupported channels show an explicit reason.
