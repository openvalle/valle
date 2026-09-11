# Timeline authoring reference

A Timeline JSON document describes when footage, images, animation, audio and
captions appear in a video. This guide covers the public authoring format used by
`valle timeline` and stored by `valle project`.

Use the [Motion reference](motion.md) to author an animated scene and this guide to
place that scene alongside other clips. The [CLI guide](cli.md) covers installation,
command output, project revisions and media processing.

## Contents

- [First timeline](#first-timeline)
- [Document structure and resources](#document-structure-and-resources)
- [Time, trimming and playback](#time-trimming-and-playback)
- [Tracks and compositing](#tracks-and-compositing)
- [Visual clips and transforms](#visual-clips-and-transforms)
- [Keyframes and crossfades](#keyframes-and-crossfades)
- [Audio and video sound](#audio-and-video-sound)
- [Captions](#captions)
- [Motion integration](#motion-integration)
- [Lottie](#lottie)
- [Adjustments](#adjustments)
- [Validation and troubleshooting](#validation-and-troubleshooting)
- [Schemas and verification](#schemas-and-verification)

## First timeline

Save as `cuts.timeline.json`. It shows blue for 1.5 seconds, then teal for another
1.5 seconds, at 640×360 and 30 FPS. No external resources are needed.

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "tracks": {
    "visual": [{ "clips": [
      { "kind": "solid", "color": "#2563eb", "start": 0, "duration": 1.5 },
      { "kind": "solid", "color": "#0d9488", "start": 1.5, "duration": 1.5 }
    ] }]
  }
}
```

With `valle` on PATH (use `./dist/bin/valle` in a source checkout):

```sh
valle timeline check cuts.timeline.json --json
valle timeline render cuts.timeline.json --frame 44 -o before-cut.png --json
valle timeline render cuts.timeline.json --frame 45 -o after-cut.png --json
valle timeline render cuts.timeline.json -o cuts.mp4 --events
```

Frame indices are zero-based. Frame 44 belongs to the blue clip; frame 45 belongs
to the teal clip. The output has 90 video frames. Output paths must be new files.
`--frame` requires PNG; without it, the command requires MP4.

The document controls dimensions, FPS and duration. The current Timeline CLI uses
Native Raster composition and does not expose Motion's `--size`, `--duration`,
`--backend` or `--workers` options. MP4 encoding and media decoding need compatible
FFmpeg shared libraries; see [runtime setup](cli.md#runtime-notes).

## Document structure and resources

The root has exactly three public fields:

| Field | Required | Meaning |
| --- | --- | --- |
| `canvas` | Yes | Output width, height, FPS and optional background |
| `resources` | No | Resource alias → locator string |
| `tracks` | Yes | Arrays of visual, audio, caption and adjustment tracks |

Do not add a root `version`, `duration`, generated clip IDs, internal `document`
envelopes or render-package fields. Track objects contain `clips`; caption tracks
also have `style` and optional `layout`. Empty track bands can be omitted.
The public format rejects unknown fields instead of treating them as extension
data. Motion `props` is a separately typed input surface, described below.

### Canvas

| Field | Contract |
| --- | --- |
| `width`, `height` | Positive integer pixel dimensions |
| `fps` | Positive JSON number, or canonical rational string such as `"30000/1001"` |
| `background` | Static color, default opaque black `"#000000ff"` |

Use `"#00000000"` for a transparent PNG background. MP4 delivery requires an opaque
canvas background, even if an opaque visual clip happens to cover the canvas.
Prefer even dimensions for ordinary H.264 delivery and verify the chosen encoder.
The current normalized audio output is 48 kHz stereo; there are no public
`sampleRate` or `channelLayout` canvas fields.

The total duration is the maximum `start + duration` over **all four track bands**.
Audio or an adjustment extending past the last visual clip therefore extends the
export. During uncovered visual intervals, the canvas background remains visible.
There must be a positive total duration.

### Named resources

Resource values are locator strings, not objects containing MIME types or duration:

```json
{
  "footage": "media/interview.mp4",
  "footageAudio": "media/interview.mp4",
  "poster": "media/poster.png",
  "captionFont": "fonts/NotoSans-Regular.ttf",
  "title": "title.motion.tsx"
}
```

This is a `resources` fragment, not a complete Timeline. A clip references the
alias, for example `"src": "footage"`. Caption `style.font` and Motion `component`
also reference aliases. A filesystem path is not a replacement for an alias in
those fields.

Aliases contain 1–64 ASCII letters, digits, dots, underscores or hyphens. Locators
must be non-empty and contain no whitespace/control characters. For a directory
or filename containing spaces, use a suitable path without spaces; a JSON-escaped
space still becomes a space after decoding.

Local relative paths resolve from the **Timeline file's directory**, independent
of the shell's current directory. Absolute filesystem paths also work. The CLI
can fetch HTTP(S) resources into its cache; a valid URL alone does not prove that
the server is reachable or the payload is a supported asset. Use ordinary local
paths rather than `file://` URLs. For Motion components and their local imports,
keep the source tree locally available; Motion asset bindings currently require
local locators.

Use separate aliases when the same file serves different media roles, especially
video and audio. Keep only needed resources: the current CLI reads the declared
resource map while preparing a render, including entries not active at the
requested frame. A missing unused file can still prevent rendering.

## Time, trimming and playback

Timeline time fields are **non-negative JSON numbers in seconds**, normalized to
six decimal places. `1.5` is valid; `"1.5"`, `"3/2"` and frame-count objects are
not time values. FPS is the exception that also accepts a rational string.

| Field | Time domain and default |
| --- | --- |
| `start` | Required placement on the output timeline, in seconds |
| `duration` | Required positive length on the output timeline, in seconds |
| `trimStart` | Source offset, default `0`; video, audio, Lottie and Motion |
| `rate` | Positive playback multiplier, default `1`; cannot animate or be negative |
| `end` | Source boundary policy: `error` (default), `hold`, `loop` |
| Motion `sourceDuration` | Positive duration of the authored Motion source, default clip `duration` |

For an active clip at output time `t`, before applying the boundary policy:

```text
clipTime   = t - start
sourceTime = trimStart + clipTime × rate
```

A clip with `start: 2`, `duration: 3`, `trimStart: 4`, `rate: 2` occupies output
seconds `[2,5)` and requests source seconds `[4,10)`. Changing `rate` does not
automatically change `duration`. Set both deliberately.

Clip intervals include the start and exclude the end. Author times are converted
to the output frame/sample grid during rendering. Prefer frame-aligned boundaries
when exact cuts matter; at 30 FPS, 1.5 seconds is frame 45. Times shorter than one
frame and rounded boundaries may not produce the visual duration their decimal
precision suggests.

`end: "error"` validates the requested source range against the prepared asset.
Keep `trimStart + duration × rate` within its usable duration. `timeline check`
opens the resources used by the document and validates these ranges.

`hold` retains the source's ending state after its boundary. For audio, it holds
the final sample; it does **not** mean silence. End an audio clip at the desired
time or fade its gain when silence is intended.

`loop` wraps against the **whole source duration**, including the initial
`trimStart` offset. It does not define a repeating subrange beginning at
`trimStart`. For example, an offset into a three-second source wraps back to source
time zero at its end. Prepare a trimmed asset first when only a subrange should loop.

## Tracks and compositing

| Band | Clip content | Behavior |
| --- | --- | --- |
| `visual` | `solid`, `image`, `video`, `lottie`, `motion` | Later tracks paint over earlier tracks |
| `audio` | Media source, gain and pan; no `kind` field | Active tracks mix together |
| `caption` | Text or rich runs, layout and animation; no `kind` field | Composited above the visual result; later caption tracks are above earlier ones |
| `adjustment` | Currently `color-grade` | Applied to the visual result before captions |

Within each track, list clips in chronological order with
`next.start >= previous.start + previous.duration`. Overlaps and out-of-order
clips are rejected. A gap is implicit; do not insert internal gap objects.
For simultaneous clips, create separate tracks. These rules also apply to audio,
caption and adjustment tracks.

JSON object key order does not change the band order. A caption stays above the
visual bands even if the `caption` key is written before `visual`. Use Motion text
inside a visual clip when text needs to participate in a different visual stack.

There is no public `transition` clip or transition object. A visual crossfade uses
two tracks and an opacity curve; an audio crossfade uses two tracks and gain curves.
The next section includes a complete visual example.

## Visual clips and transforms

Every visual clip requires `kind`, `start`, `duration`, plus its source-specific
fields:

| Kind | Required source fields | Optional source fields |
| --- | --- | --- |
| `solid` | `color` | None; a generated canvas-sized color surface |
| `image` | `src` | `fit` |
| `video` | `src` | `trimStart`, `rate`, `end`, `fit` |
| `lottie` | `src` | `trimStart`, `rate`, `end`, `fit` |
| `motion` | `component` | `trimStart`, `sourceDuration`, `rate`, `end`, `props`, `cues`, `resources`, `phases` |

`solid` is a color, not a placeholder requiring an image file. Its `color` is
static; use separate solids and opacity curves or Motion for an animated color.

### Common visual properties

| Property | Default | Meaning |
| --- | --- | --- |
| `position` | `[0.5,0.5]` | Position of the source anchor in canvas-width/height units |
| `size` | Canvas width/height | Positive target rectangle in canvas pixels |
| `scale` | `[1,1]` | Multipliers on the target width/height |
| `rotation` | `0` | **Degrees**, without automatic shortest-angle wrapping |
| `anchor` | `[0.5,0.5]` | Static source-relative anchor; `[0,0]` is its top-left |
| `opacity` | `1` | Composed clip opacity, 0–1 |
| `blend` | `normal` | Static blend mode with the existing backdrop |

`position`, `size`, `scale`, `rotation` and `opacity` can be constants or keyframe
curves. `anchor` and `blend` are static. Position uses normalized canvas coordinates;
size uses pixels; rotation uses degrees (`90` is a quarter turn).

The target rectangle defaults to the canvas. Use `size: [320,180]` for a smaller
rectangle; `position` places its anchor and `scale` further scales it. Negative
scale components mirror an axis; a linear scale curve must not cross zero.

`fit` accepts `contain` (default), `cover`, `fill`, `none`. It controls sampling in
the target rectangle: `contain` preserves the full image with possible empty space,
`cover` crops to fill, `fill` stretches, and `none` keeps the source's natural size.
An image/video with `fit: "cover"` and no explicit size fills the canvas.

Blend modes: `normal`, `screen`, `lighten`, `color-dodge`, `multiply`, `darken`,
`color-burn`, `linear-burn`, `overlay`, `soft-light`, `hard-light`, `difference`,
`exclusion`, `hue`, `saturation`, `color`, `luminosity`.

### Image placement example

Save as `image.timeline.json` beside a **640×360** `poster.png`. Two copies occupy
the left and right halves of the canvas. The second is slightly rotated.

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30, "background": "#0f172a" },
  "resources": { "poster": "poster.png" },
  "tracks": { "visual": [
    { "clips": [{ "kind": "image", "src": "poster", "start": 0, "duration": 3,
      "position": [0.25,0.5], "scale": [0.45,0.45] }] },
    { "clips": [{ "kind": "image", "src": "poster", "start": 0, "duration": 3,
      "position": [0.75,0.5], "scale": [0.45,0.45], "rotation": 4.6 }] }
  ] }
}
```

For arbitrary masks, CSS filters, complex layout or text graphics, author a Motion
component and place it as a Motion clip. The public Timeline visual clip does not
expose `style`, `width`, `height`, `mask`, `filters` or a nested `transform` object.

## Keyframes and crossfades

Constants are written directly. An animated parameter uses:

```json
{
  "keyframes": [[0, 0, "ease-out"], [0.4, 1], [2.6, 1], [3, 0]],
  "interpolation": "linear"
}
```

This is a parameter fragment. Each keyframe is `[time,value]` or
`[time,value,easing]`. The easing belongs to the interval **leaving** that keyframe;
the last keyframe must not include easing. Use arrays for vector values, for
example `[0,[0.25,0.5]]`.

Ordinary visual transforms, audio gain/pan and caption presentation curves use
**clip-local seconds**, starting at zero regardless of the clip's `start`.
Motion `props` curves use **Motion source seconds**, so trimming/rate/looping affect
them. This distinction matters when retiming a Motion clip.

Keyframes must be non-empty, strictly increasing and lie in the owning duration
(the last time may equal that duration). Values before the first and after the
last keyframe clamp to the endpoint. `interpolation` is `linear` by default;
`step` holds each keyframe's value until the next one. Values must remain valid
for their field, including values produced by easing.

Named easing values are `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`.
These Timeline names differ from Motion's helper spelling. A custom easing is
`{"type":"cubic-bezier","x1":0.25,"y1":0.1,"x2":0.25,"y2":1}`; both x
control points must be in 0–1. Easing is useful for linear segments; omit it for
step interpolation.

### Crossfade example

Save as `crossfade.timeline.json`. Blue stays opaque underneath while teal fades
in on the upper track during output seconds 1–2. The upper clip's local time zero
is output second 1.

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "tracks": { "visual": [
    { "clips": [{ "kind": "solid", "color": "#2563eb", "start": 0, "duration": 2 }] },
    { "clips": [{ "kind": "solid", "color": "#0d9488", "start": 1, "duration": 2,
      "opacity": { "keyframes": [[0,0],[1,1]] } }] }
  ] }
}
```

For two fully opaque sources, fading only the upper one preserves coverage during
the overlap. Fading both visual clips out/in independently with ordinary
source-over blending can expose the background and produce an unwanted dark dip.
For audio, complementary gain ramps on the two tracks produce a linear crossfade.

## Audio and video sound

A `kind: "video"` clip in `tracks.visual` plays its selected source audio stream
by default. Its `gain` is a finite, non-negative amplitude multiplier: `0` mutes,
`1` preserves the original level, and `2` doubles it. Gain also accepts clip-local
keyframe curves. Trim, playback rate and placement follow the video automatically.
A video without audio stays silent. Mono and stereo inputs retain their channel layout.

Use `tracks.audio` for music, voiceover or independently edited sound. Adding an
explicit audio clip for the same footage mixes it again; set the video's `gain`
to `0` when replacing its original sound with that independently edited track.
An audio clip has no `kind` field and can use the same resource alias as the video.

| Audio field | Contract |
| --- | --- |
| `src`, `start`, `duration` | Required resource alias, placement and output duration |
| `trimStart`, `rate`, `end` | Same source-time mapping as video |
| `gain` | Constant/curve, default `1`, finite and non-negative; a linear multiplier, not dB |
| `pan` | Constant/curve, default `0`; `-1` left, `0` center, `1` right |

`gain: 0` is silent; `gain: 0.5` halves amplitude. Multiple audio tracks add their
signals, so choose gain values with room for the combined result. Pan linearly
attenuates the opposite channel. The public audio clip has no effect-chain,
automatic loudness-normalization or pitch-preservation option; do not assume
changing `rate` performs speech time-stretching without a pitch change.

### Footage, original sound and music

Save as `media.timeline.json`. Provide a 640×360 `input.mp4` with picture and audio
lasting at least four seconds, plus a `music.wav` file. The edit uses source seconds
0.5–3.5 and mixes quieter looping music with fade-in/out.

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "resources": {
    "footage": "input.mp4", "music": "music.wav"
  },
  "tracks": {
    "visual": [{ "clips": [{ "kind": "video", "src": "footage",
      "start": 0, "duration": 3, "trimStart": 0.5, "gain": 0.8 }] }],
    "audio": [
      { "clips": [{ "src": "music", "start": 0, "duration": 3, "end": "loop",
        "gain": { "keyframes": [[0,0],[0.3,0.15],[2.5,0.15],[3,0]] }, "pan": 0 }] }
    ]
  }
}
```

```sh
valle timeline check media.timeline.json --json
valle timeline render media.timeline.json -o media.mp4 --events
```

Previewing a PNG cannot verify the mix. Export the video and listen to it; check
both the original sound and the music, especially around cuts and fades.

## Captions

A caption track requires `style` and `clips`. The style's `font` is a resource
alias for an actual font file, not an operating-system family name. The examples
below use a local `caption.ttf`; copy a font whose license permits your use. In a
source checkout, the [font inventory](../assets/fonts/README.md) lists bundled faces.

### Style and layout

| Field | Default / meaning |
| --- | --- |
| Track `style.font` | Required font resource alias |
| `style.fontSize` | `64` canvas pixels; choose a size appropriate to the canvas |
| `style.color` | Opaque white `"#ffffffff"` |
| `style.shadow` | Optional `{ color, offset: [x,y], blur? }`; pixel offsets and non-negative blur sigma, default blur `0` |
| Track/clip `layout.region` | `[0.1,0.76,0.8,0.18]`; normalized `[x,y,width,height]`, contained within the canvas |
| Track/clip `layout.align` | `bottom-center`; anchors text within the region |

Clip layout overrides the corresponding track layout fields independently. Align
values are `top-left`, `top-center`, `top-right`, `center-left`, `center`,
`center-right`, `bottom-left`, `bottom-center`, `bottom-right`.

Each caption clip requires `start`, `duration`, and exactly one of `text` or a
non-empty `runs` array. A run has `text` plus optional `fontSize` and `color`.
Spaces/newlines are literal content: include spaces at run boundaries and use
`\n` for an explicit line break. There is no arbitrary per-run CSS or run font
override. Timed run `start`/`end` fields are reserved for karaoke.

### Preset animation example

Save as `captions.timeline.json` beside `caption.ttf`:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30, "background": "#102030" },
  "resources": { "font": "caption.ttf" },
  "tracks": {
    "caption": [{
      "style": { "font": "font", "fontSize": 36, "color": "#ffffff",
        "shadow": { "color": "#00000080", "offset": [0,2], "blur": 4 } },
      "layout": { "region": [0.08,0.2,0.84,0.6], "align": "center" },
      "clips": [{ "start": 0, "duration": 3,
        "runs": [{ "text": "Edit with " }, { "text": "Valle", "color": "#38bdf8" }],
        "enter": { "preset": "slide-up" },
        "display": { "preset": "breathe", "rate": 1 },
        "exit": { "preset": "fade" }
      }]
    }]
  }
}
```

`enter`, `display` and `exit` can each be omitted. Presets use objects only:
`"enter": { "preset": "fade", "duration": 0.3 }`. Enter/exit `duration` is optional
and defaults to the preset's recommended duration. Display uses
`{ "preset": "breathe", "rate": 1 }`; its optional rate defaults to `1`.
String shorthand such as `"enter": "fade"` is rejected.

| Phase | Presets |
| --- | --- |
| Enter | `fade`, `slide-up`, `slide-down`, `slide-left`, `slide-right`, `pop`, `zoom-in`, `zoom-out`, `focus`, `wipe-right`, `wipe-left` |
| Display | `breathe`, `float`, `sway`, `pulse`, `shake` |
| Exit | `fade`, `slide-up`, `slide-down`, `slide-left`, `slide-right`, `pop`, `zoom-in`, `zoom-out`, `focus`, `wipe-right`, `wipe-left` |

Omitted preset durations use the catalog defaults: fade enters in 0.2 seconds and
exits in 0.16 seconds; other defaults are in the
[preset catalog](../crates/valle-timeline/src/caption_presets.rs). Inspect short captions where
the requested enter/exit phases compete for time.

### Custom presentation

Use `presentation` for custom curves. It cannot coexist with `enter`, `display` or
`exit` on the same caption clip.

| Property | Default and units |
| --- | --- |
| `opacity` | `1`, range 0–1 |
| `translation` | `[0,0]`, **canvas pixels**, unlike visual `position` |
| `scale` | `1`, non-negative scalar, unlike the visual scale pair; opacity is the clearest way to hide text |
| `rotation` | `0`, degrees |
| `clipInset` | `[0,0,0,0]`, normalized top/right/bottom/left text-block insets |
| `blur` | `0`, non-negative blur sigma in canvas pixels |

All six properties accept constants or clip-local curves. Opposing clip insets
must leave a positive visible rectangle. This presentation fragment fades and
slides a three-second caption into position:

```json
{
  "opacity": { "keyframes": [[0,0],[0.4,1],[2.6,1],[3,0]] },
  "translation": { "keyframes": [[0,[0,24],"ease-out"],[0.4,[0,0]]] }
}
```

### Karaoke and scrolling

Karaoke uses `behavior: { "type": "karaoke", "mode": "word" }` or mode `line`,
plus explicitly timed runs. Run times are clip-local seconds, must be supplied
as start/end pairs, and satisfy `0 <= start < end <= duration`. Runs must be
chronological and non-overlapping. Timings are not inferred from audio.
The current renderer brightens an entire run when its start is reached; it does
not sweep continuously across that run during its window. Use one run per word
for word-level timing. Line mode extends the reveal to the next explicit newline.
Unrevealed text remains dimmed rather than disappearing.

Save as `karaoke.timeline.json` beside `caption.ttf`:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30, "background": "#102030" },
  "resources": { "font": "caption.ttf" },
  "tracks": { "caption": [{
    "style": { "font": "font", "fontSize": 40, "color": "#38bdf8" },
    "layout": { "region": [0.08,0.2,0.84,0.6], "align": "center" },
    "clips": [{ "start": 0, "duration": 3,
      "behavior": { "type": "karaoke", "mode": "word" },
      "runs": [
        { "text": "Build ", "start": 0, "end": 0.7 },
        { "text": "your ", "start": 0.7, "end": 1.4 },
        { "text": "story", "start": 1.4, "end": 2.5 }
      ]
    }]
  }] }
}
```

Scroll uses `behavior: { "type": "scroll", "axis": "horizontal", "speed": 80 }`.
Axis can also be `vertical`. Speed is signed canvas pixels per second; positive
horizontal speed moves toward the right and positive vertical speed moves upward.
Scrolling wraps after the text/region traversal and clips to the caption region.
Only one behavior object is allowed per clip. Use custom presentation or Motion
when a different text animation is needed.

## Motion integration

Declare a local `.motion.tsx` file in root `resources`, then reference its alias
using `component`. The Motion source's default function provides the scene; the
Timeline alias does not have to match that function's name.

| Motion field | Meaning |
| --- | --- |
| `props` | Declared Motion prop name → constant or typed source-time curve |
| `resources` | Declared Motion asset slot → root resource alias |
| `cues` | Declared cue name → `source-range` binding |
| `phases` | Optional `enterDuration`, `exitDuration`, in source seconds |
| `sourceDuration` | Source animation duration; also bounds source-time props/cues |

Prop names/types must match `defineControls`. Required bindings must be available;
unknown props or incompatible curve values are not arbitrary metadata. Root
`resources` locates files; clip `resources` connects those files to named Motion
asset slots. Inside the JSX, an image slot `hero` is read as `asset://hero`.

Timeline supplies Motion props using the same color and unit vocabulary as JSX:

| Motion control | Timeline value |
| --- | --- |
| `number` | Number within the declared range; curves allowed |
| `length` | Pixels as a number or `"24px"`; curves allowed |
| `angle` | Degrees as a number or an explicit `"15deg"`, `"0.5rad"`, `"0.25turn"`; curves allowed |
| `point` | `[x,y]`; curves allowed |
| `rect` | `[x,y,width,height]`; curves allowed |
| `color` | CSS color such as `"#38bdf8"`; curves interpolate color channels |
| `string`, `boolean`, `select` | Constant string/boolean/allowed select string; no curves |

For example, use `"#38bdf8"` both in a JSX color default and in a Timeline color prop. `path` props are not admitted through this
Timeline binding path; `nodeTarget` has no completed Native conversion here.
Leave those controls to a supported host integration instead of assuming every
Motion control has a usable Timeline override.

The current Timeline CLI prepares Motion at the Timeline canvas size. It does not
accept a clip `data` field or a `--data` binding; a component requiring external
structured data through Motion's fourth argument cannot be supplied that data
through this Timeline command. Use supported props/assets or a suitable host
integration for that input.

### Complete Motion overlay example

Save as `overlay.motion.tsx`:

```tsx
export const controls = defineControls({
  props: {
    title: string({ default: "Hello, Valle" }),
    accent: color({ default: "#38bdf8" }),
    amount: number({ default: 0, min: 0, max: 1 }),
  },
  cues: { reveal: spanCue({ required: true }) },
  assets: { hero: asset({ kind: "image", required: true }) },
});

export default function Overlay(ctx, props, signals) {
  const opacity = interpolate(signals.reveal.progress, [0,0.3], [0,1]);
  return (
    <Scene className="relative h-full w-full">
      <Image src="asset://hero" style={{ position: "absolute", left: 32, top: 32,
        width: 160, height: 90, objectFit: "cover", borderRadius: 12 }} />
      <View style={{ position: "absolute", left: 32, bottom: 40,
        width: 576 * props.amount, height: 8, backgroundColor: props.accent }} />
      <Text style={{ position: "absolute", left: 32, top: 170,
        fontSize: 40, color: "#ffffff", opacity }}>{props.title}</Text>
    </Scene>
  );
}
```

Save as `motion.timeline.json` beside that source and `poster.png`:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30, "background": "#102030" },
  "resources": { "overlay": "overlay.motion.tsx", "poster": "poster.png" },
  "tracks": { "visual": [{ "clips": [{
    "kind": "motion", "component": "overlay", "start": 0.5, "duration": 3,
    "sourceDuration": 3,
    "props": {
      "title": "Built on a timeline", "accent": "#38bdf8",
      "amount": { "keyframes": [[0,0],[3,1]] }
    },
    "resources": { "hero": "poster" },
    "cues": { "reveal": { "type": "source-range", "start": 0, "end": 2.5 } },
    "phases": { "enterDuration": 0.3, "exitDuration": 0.3 }
  }] }] }
}
```

```sh
valle timeline check motion.timeline.json --json
valle timeline render motion.timeline.json --frame 60 -o motion-overlay.png --json
```

The output lasts 3.5 seconds. At frame 60 (output second 2), this Motion source is
at second 1.5. Its prop curve is therefore halfway complete. The first half second
shows the canvas background.

`source-range` cue `start`/`end` are source seconds, not output placement times.
Optional cue `enterDuration`/`exitDuration` default to zero. Cues must fit the
source range; they provide `signals.name` to the component. `phases` changes
Motion's context phase timing, not the clip's placement or total output duration.

When retiming, keep the source duration explicit. For example, a three-second
Motion source played at `rate: 2` needs an output `duration: 1.5` to play once.
Setting only rate leaves the default `sourceDuration` equal to the clip duration
and can request time beyond that source. Outer clip opacity/position still use
clip-local time; its Motion props and cues follow the source clock.

Repeated clips can use the same component alias with different resource bindings.
The CLI prepares each distinct binding set internally; no duplicate aliases are needed.
Use the [Motion limitations](motion.md#troubleshooting-and-limits) when selecting
advanced effects; Timeline placement does not remove their rendering constraints.

## Lottie

Lottie clips reference a JSON animation using `src`. Timeline reads the animation's
`w`, `h`, `fr`, `ip` and `op` metadata; its duration is `(op - ip) / fr`.
Use a self-contained export. The current CLI rejects external image references
and declared font lists: embed images and convert text to shapes as appropriate.
Valid Lottie JSON does not imply support for every exporter feature.

Save as `lottie.timeline.json` beside a self-contained `badge.json`:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30, "background": "#102030" },
  "resources": { "badge": "badge.json" },
  "tracks": { "visual": [{ "clips": [{ "kind": "lottie", "src": "badge",
    "start": 0, "duration": 3, "end": "loop", "position": [0.5,0.5]
  }] }] }
}
```

As with image/video clips, set scale based on the animation's dimensions. The
Timeline FPS controls output sampling; it does not rewrite the Lottie animation.

## Adjustments

The public adjustment catalog currently contains one kind:
`{ "kind": "color-grade", "temperature": value }`, with temperature in `[-1,1]`.
Zero is neutral; positive values warm the image and negative values cool it.
Temperature is static, not a keyframe parameter. Split into adjacent adjustment
clips for discrete changes or author a suitable Motion effect for other behavior.

Save as `adjustment.timeline.json`. The second half has a warm adjustment:

```json
{
  "canvas": { "width": 640, "height": 360, "fps": 30 },
  "tracks": {
    "visual": [{ "clips": [{ "kind": "solid", "color": "#a0a0a0",
      "start": 0, "duration": 3 }] }],
    "adjustment": [{ "clips": [{ "kind": "color-grade", "temperature": 0.3,
      "start": 1.5, "duration": 1.5 }] }]
  }
}
```

Adjustments affect the composed visual result during their active intervals;
captions are drawn afterward. An adjustment track is not a per-video filter
attachment. Multiple active adjustments from different tracks apply in the
declared track/clip order. Avoid stacking them accidentally.

## Validation and troubleshooting

`timeline check` decodes the public document and compiles its structural/time
contract, then prepares used resources, compiles Motion components and opens the
same verified package as rendering. Missing used files, source overruns and invalid
Motion prop values fail this check. Unused resources are not opened. Checking does
not render a frame or verify every glyph/encoder combination.

Use PNG previews at boundaries and representative animation times, then export a
short MP4 when audio/video delivery matters. Validate input dimensions and source
duration when selecting trim/rate values. For exact JSON output, errors, progress
and exit codes see the [CLI output contract](cli.md#output-contract-for-scripts-and-agents).

| Problem | What to inspect |
| --- | --- |
| Unknown field / invalid shape | Use the sparse public format; check spelling and avoid internal IDs, `version`, `transform` and `transition` objects |
| Missing resource alias | Declare it in root `resources`; use the alias in `src`, `component` and `style.font` |
| Track overlap | Sort clips and keep each track non-overlapping; split simultaneous clips across tracks |
| Video has no sound | Check whether the source has audio and whether its video `gain` is zero |
| Video/image does not fill canvas | Use `fit: "cover"`; check any explicit `size`, `scale` and `position` |
| Motion source range failure | Check `sourceDuration`, `trimStart`, `rate`, output `duration` and `end` together |
| Animation starts at the wrong time | Distinguish output time, clip-local keyframes and Motion source-time props/cues |
| Rotation is unexpectedly large | Timeline rotation and numeric Motion angle props are degrees |
| Invalid keyframe | Strictly increasing times within the owning duration; no easing on the last keyframe |
| Caption is rejected | Exactly one of `text`/non-empty `runs`; choose presets or custom `presentation` |
| Karaoke timing error | Supply ordered clip-local run windows and `behavior.type: "karaoke"` |
| Check passes but render fails | Inspect actual resources, media extent, Motion controls, font availability and backend admission |
| MP4 rejects a transparent scene | Set an opaque `canvas.background` or use PNG when alpha is required |
| Export is longer than expected | Check the latest ending clip in every band, including audio and adjustments |

Use `project create/show/apply/history/restore` when revisions are needed; those
commands store the same public Timeline document. `project apply` accepts a
complete document and a base revision, not a raw patch. See
[project editing](cli.md#project-versioned-timeline-editing) for that workflow.

## Schemas and verification

The public contracts are derived from
[Rust wire types](../crates/valle-timeline/src/wire/timeline.rs). Generated
[TypeScript types](../crates/valle-timeline/schema/timeline.generated.ts) and the
[JSON Schema bundle](../crates/valle-timeline/schema/timeline.schema-bundle.json)
support tooling. The bundle is an envelope: select
`schemas["timeline.schema.json"]` as the Timeline root schema. The edit-request
and edit-response roots are separate. Do not validate author input against the
internal render/storage schema.

Schema validation covers shape; `timeline check` also applies runtime document
constraints, and rendering verifies the actual resources. The
[compiler tests](../crates/valle-compiler/tests/timeline.rs),
[public contract tests](../crates/valle-timeline/tests/timeline_contract.rs),
[caption preset tests](../crates/valle-compiler/tests/caption_presets.rs) and
[CLI delivery code](../crates/valle-cli/src/cmd/timeline.rs) are useful references
when updating this guide.

The nine complete Timeline examples passed `timeline check` and produced Native
Raster PNG previews. Boundary frames covered cuts, crossfades, karaoke, Motion
placement and adjustments. Cuts, media/audio, captions and Motion also exported
MP4 files. The media example used generated footage and two distinct test tones
to verify original sound, background music and the music gain ramps.

Current delivery observation: on the tested local build, `ffprobe` reports a
video-stream duration of 2.966667 seconds for the 90-frame exports, and 3.466667
for the 105-frame export. All frame packets are present with 1/30-second durations;
the video-stream duration metadata is one frame shorter than their presentation
extent. This discrepancy remains a delivery follow-up, not a verified frame loss.

Additional probes covered custom caption presentation, scrolling, line karaoke,
rational FPS, mirroring, hold/loop and Motion retiming. Negative probes checked
invalid shapes/times, overlaps, curves, captions and resources, including malformed preset objects. The CLI regression suite covers explicit preset
durations, resource preparation, video volume, stereo separation and Motion bindings.

External-file examples require the explicitly named files; parameter/resource
fragments are not standalone timelines. Lottie verification used a self-contained
shape animation. These results do not claim coverage of every Lottie exporter,
font, remote locator, color pipeline or Web renderer combination. Verification
scripts and generated media stay outside the public examples directory.
