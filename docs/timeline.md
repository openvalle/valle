# Timeline documents

A Timeline is a JSON document, `*.timeline.json`, that cuts footage, images, Motion scenes,
sound and captions together. `valle timeline check` validates it and `valle timeline render`
draws it. Unknown fields are errors, and every error names the field and what it takes, so
`check` is the quickest way to learn a field you are unsure of.

Every timeline file uses the `.timeline.json` suffix, including nested timelines. Ordinary
data and configuration keep `.json`. A new project starts with `main.timeline.json` and a
`project.json` that names it.

## A first timeline

```json timeline
{
  "canvas": { "width": 1920, "height": 1080, "fps": 30 },
  "resources": {
    "title": "title.motion.tsx",
    "footage": "footage.mp4",
    "music": "music.wav"
  },
  "tracks": {
    "visual": [
      {
        "items": [
          { "id": "intro", "kind": "motion", "component": "title", "duration": 2.5 },
          { "transition": "fade", "inOffset": 0.5, "outOffset": 0.5 },
          { "id": "shot", "kind": "video", "src": "footage", "trimStart": 1, "duration": 1.5 }
        ]
      }
    ],
    "audio": [{ "clips": [{ "src": "music", "start": 0, "duration": 4, "gain": 0.5 }] }]
  }
}
```

```sh
valle timeline check edit.timeline.json
valle timeline render edit.timeline.json --frame 60 -o frame.png   # look at one frame
valle timeline render edit.timeline.json -o edit.mp4
```

A visual track plays its clips one after another (`items`), or each at its `start` (`clips`).
Later visual tracks draw on top. The document's length is where its last clip ends.

## Document

| Field | Required | What it is |
| --- | --- | --- |
| `canvas` | yes | `{ width, height, fps, background }`, below |
| `tracks` | yes | `visual`, `audio`, `caption` and `adjustment`: lists of tracks |
| `resources` | no | Files, by the name clips use for them |
| `name` | no | A label |
| `startTimecode` | no | The timecode of the first frame, such as `"01:00:00:00"`; a label only |
| `markers` | no | `[{ at, duration, name, color, comment, metadata }]`; `at` is required |
| `metadata` | no | Anything; Valle keeps it and `timeline query --meta` searches it |
| `mix` | no | Sound buses and the master: see the sound section |

The canvas:

- `width`, `height`: whole pixels, 1 to 16384.
- `fps`: a number such as `30`, or a ratio as text, such as `"30000/1001"`.
- `background`: a CSS color, `#000000ff` (opaque black) unless given. An MP4 needs an opaque
  one; a transparent background renders to `.mov` (ProRes 4444) or PNG.

All times are **seconds**, as numbers: `2.5`, not `"00:00:02:12"`. A time is snapped to the
frames when the timeline is drawn.

## Resources

`resources` names the files clips use; clips refer to them by that name, never by path.

```json
"resources": {
  "logo": "images/logo.png",
  "title": "title.motion.tsx",
  "interview": { "path": "/Volumes/shoot/A001.mov", "timecode": "10:00:00:00", "fps": 25 },
  "brand": ["fonts/Inter-Regular.ttf", "fonts/Inter-Bold.ttf"]
}
```

- A path is relative to the timeline file's folder, or absolute. `https://` addresses work
  and are downloaded once into the cache.
- An object adds the source's own `timecode`, `fps`, `duration` and a `name` label.
- A list is a font family's files, for captions.
- Names are 1 to 64 ASCII letters, digits, dots, underscores or hyphens.

In a project, `valle project add` copies a file in and adds its resource.

## Tracks

```json
"tracks": {
  "visual": [{ "id": "main", "items": [] }, { "clips": [] }],
  "audio": [{ "clips": [], "gain": 0.8 }],
  "caption": [{ "clips": [], "style": { "fontSize": 64 } }],
  "adjustment": [{ "clips": [] }]
}
```

Each track has either:

- `items`: a **sequence**. Its clips follow one another from 0 s, so they have no `start`. A
  `{ "gap": 1.5 }` item leaves time empty, and a transition item goes between two clips.
  Clips in a sequence need an `id`.
- `clips`: clips placed by their own `start`. They must not overlap on one track: put clips
  that play at the same time on separate tracks.

A track's `id` is `visual0`, `audio1` and so on unless given. Track fields:

| Track | Fields |
| --- | --- |
| visual | `id`, `items` or `clips`, `gain`, `pan` (for its videos' own sound) |
| audio | `id`, `items` or `clips`, `gain`, `pan`, `effects`, `sends`, `output` |
| caption | `id`, `clips`, `style` or `presenter` |
| adjustment | `id`, `clips` |

Every track also takes `enabled`: `false` keeps the track and its clips but does not show or
play them, as hiding or muting a track in an editor does.

## Clips

Every clip has:

| Field | What it is |
| --- | --- |
| `id` | A name for edits, anchors and queries; required in a sequence; unique in the document |
| `start` | Seconds from 0, or an anchor (below); not in a sequence |
| `duration` | Seconds, above 0 |
| `enabled` | `false` keeps the clip but does not show or play it |
| `name`, `markers`, `metadata` | Labels Valle keeps |

A visual clip has a `kind`:

| `kind` | Its own fields |
| --- | --- |
| `solid` | `color`: a CSS color |
| `image` | `src`: an image resource (PNG, JPEG, WebP, SVG) |
| `video` | `src`, and the source fields below; `gain`, `pan` for its own sound |
| `motion` | `component`: a Motion scene resource; `props`, `data`, `resources` for it |
| `timeline` | `src`: another timeline document, nested; its sound plays too |

An audio clip has `src` and the source fields; a caption clip has `text` or `runs`; an
adjustment clip has `kind`: `color-grade` or `motion`.

### Source time

For `video`, `audio` and `timeline` clips, and `motion` clips (without `timeMap`):

- `trimStart`: where in the source the clip begins, in seconds.
- `rate`: playback speed, above 0; `2` plays twice as fast.
- `timeMap`: `[[clip time, source time], …]`, two points or more, instead of `trimStart` and
  `rate`; for ramps, holds and reverse play.
- `end`: what happens past the source's end: `error` (the default; `check` reports it),
  `hold` the last frame, or `loop`.

```json
{ "id": "slow", "kind": "video", "src": "footage", "start": 0, "duration": 4,
  "trimStart": 1, "rate": 0.5 }
```

### Anchors

A clip's `start` can follow another clip: `{ "clip": "shot", "offset": 0.5 }` starts it 0.5 s
after the start of `shot`, on any track. Moving `shot` moves it too. Anchors are how sound,
titles and captions stay with the picture they belong to.

```json
{ "src": "whoosh", "start": { "clip": "shot", "offset": -0.2 }, "duration": 1 }
```

## Placing visual clips

A visual clip fills the canvas unless told otherwise:

| Field | Default | What it is |
| --- | --- | --- |
| `position` | the canvas's center | `[x, y]` in pixels: where the anchor point goes |
| `size` | the canvas's size | `[width, height]` in pixels: the clip's box |
| `anchor` | `[0.5, 0.5]` | The point of the box that `position` places: `[0, 0]` its top left |
| `fit` | `contain` | How content fills the box: `contain`, `cover`, `fill` or `none` |
| `scale` | `[1, 1]` | `[x, y]` around the anchor; negative flips |
| `rotation` | `0` | Degrees, clockwise, around the anchor |
| `opacity` | `1` | From 0 to 1 |
| `blend` | `normal` | `multiply`, `screen`, `overlay`, `darken`, `lighten`, `color-dodge`, `color-burn`, `hard-light`, `soft-light`, `difference`, `exclusion`, `hue`, `saturation`, `color`, `luminosity`, `plus-lighter` |
| `effects` | none | `[{ "kind": "motion", "component": … }]`: a Motion effect over the clip |

A picture in picture, a quarter of the canvas, in the top right:

```json
{ "id": "pip", "kind": "video", "src": "footage", "start": 0, "duration": 5,
  "position": [1860, 60], "anchor": [1, 0], "size": [480, 270], "fit": "cover" }
```

## Keyframes

`position`, `size`, `scale`, `rotation` and `opacity`, a track's or clip's `gain` and `pan`,
and a Motion clip's `props` take keyframes instead of a value:

```json
"opacity": { "keyframes": [[0, 0, "ease-out"], [0.5, 1], [4.5, 1, "ease-in"], [5, 0]] }
```

- Each keyframe is `[time, value]` or `[time, value, easing]`. Times are seconds from the
  clip's start and rise strictly.
- The easing shapes the stretch leaving that keyframe, so the last one has none. It is a CSS
  easing: `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`, `cubic-bezier(…)`,
  `steps(…)`.
- `"interpolation": "step"` holds each value until the next keyframe.

## Transitions

A transition is an item of a sequence, between two clips:

```json
{ "transition": "wipeLeft", "inOffset": 0.25, "outOffset": 0.25, "params": { "softness": 0.2 } }
```

It plays from `inOffset` seconds before the cut to `outOffset` after it; both are 0 unless
given, and one must be above 0. The clips on either side must have that much source to show
past the cut. The built-in ones and their `params`:

| Transition | Params |
| --- | --- |
| `fade` | |
| `wipeLeft`, `wipeRight` | `softness` |
| `circleOpen` | `centerX`, `centerY`, `softness` |
| `simpleZoom` | `quickness`, `centerX`, `centerY` |
| `crossWarp` | |
| `linearBlur` | `intensity` |
| `directionalWarp` | `directionX`, `directionY`, `softness` |
| `dreamyZoom` | `strength`, `centerX`, `centerY` |
| `ripple` | `frequency`, `speed`, `amplitude` |
| `flyEye` | `size`, `frequency`, `colorSeparation` |
| `multiplyBlend` | `midpoint` |
| `perlin` | `scale`, `smoothness` |
| `motion` | a Motion transition: `component`, `props`, `data`, `resources` |

An unknown param is an error that lists the transition's params. On an audio track the only
transition is `crossfade`, which fades the two clips with equal power.

## Motion clips

A `motion` clip shows a Motion scene, a `.motion.tsx` resource:

```json
{ "id": "lower", "kind": "motion", "component": "lowerThird", "start": 2, "duration": 4,
  "props": { "name": "Ada Lovelace", "role": "Engineer" },
  "resources": { "photo": "portrait" } }
```

- `props` and `data` bind the scene's `controls.props` and `controls.data`; props take
  keyframes.
- `resources` binds each of the scene's `controls.resources` to a resource of the timeline,
  by name.
- The scene plays from its own 0 s at the clip's start; `trimStart`, `rate` and `end` work as
  for a video. The scene's own size is fitted into the clip's box.

A scene declares how a timeline uses it with `export const role = …`:

| Role | Used as | What it reads |
| --- | --- | --- |
| none | a `motion` clip, played as it is | |
| `overlay({ intro, outro, hold })` | a `motion` clip of any length: a title, a lower third | its intro and outro keep their length; `hold` is `once`, `loop` or `stretch` for the middle |
| `transition()` | a `motion` transition | `input://from` and `input://to` |
| `effect()` | a clip's `effects`, or an adjustment clip | `input://source`: the clip, or all under it |
| `captionPresenter({ intro, outro })` | a caption track's `presenter` | each caption as its `caption` prop |

See `valle docs motion --section timeline-roles`.

## Sound

A video plays its own sound; set the clip's `gain` to `0` to mute it. Audio tracks play
sound files.

- `gain` is an amplitude factor: `1` as recorded, `0.5` about 6 dB down, `0` silent.
- `pan` is from `-1` (left) to `1` (right).
- An audio track's `effects` are applied in order: `{ "type": "eq", "bands": [...] }`,
  `compressor`, `reverb`, `limiter`. Levels in effects are decibels (`thresholdDb`,
  `gainDb`), times seconds.
- `sends` and `output` route a track to buses that `mix.buses` declares; `mix.master` has the
  master's `effects`, `gain` and `pan`.

```json
"audio": [
  { "id": "music", "gain": 0.4, "clips": [{ "src": "music", "start": 0, "duration": 8 }],
    "effects": [{ "type": "compressor", "thresholdDb": -18, "ratio": 3 }] }
]
```

A render into `.wav` writes the sound alone. A video's result reports its loudness (LUFS)
and true peak.

## Captions

A caption track's clips each show a caption, as `text` or as timed `runs`:

```json
"caption": [
  {
    "style": { "fontFamily": "brand", "fontSize": 56, "position": "bottom", "animation": "karaoke" },
    "clips": [
      { "start": 0.5, "duration": 2, "text": "Hello there" },
      { "start": 3, "duration": 2, "runs": [
        { "text": "Word ", "start": 0, "end": 0.6 },
        { "text": "by word", "start": 0.6, "end": 1.8, "style": { "textColor": "#FFD23F" } }
      ] }
    ]
  }
]
```

- Run times are seconds from the caption's start, in order, within its duration.
- `style` sets the standard caption's look: `fontFamily` (a font resource's name),
  `fontSize`, `fontWeight`, `fontStyle`, `lineHeight`, `letterSpacing`, `wordSpacing`,
  `textAlign`, `textDecoration`, `textColor`, `highlight`, `strokeWidth`, `strokeColor`,
  `shadow`, `textShadow`, `background` (`box`, `bar`, `none`), `backgroundColor`, `position`
  (`bottom`, `top`, `middle`), `margin`, `maxWidth`, `animation` (`karaoke`, `typewriter`,
  `fade`, `pop`, `slide`, `none`) and `transition` (seconds in and out). A run's `style`
  takes the text properties only.
- Instead of `style`, `presenter: { "component": … }` draws the captions with your own Motion
  scene, whose role is `captionPresenter`.

## Adjustments

An adjustment track's clips change what is drawn below them while they play:

```json
"adjustment": [{ "clips": [
  { "kind": "color-grade", "start": 0, "duration": 8, "temperature": 0.3 }
] }]
```

`color-grade` takes `temperature` from -1 (cool) to 1 (warm). `motion` applies a Motion
effect scene to everything below.

## Nesting

A `timeline` clip plays another document inside this one, fitted into its box, with its
sound. Its `trimStart`, `rate` and `end` work as for a video. A document cannot contain
itself.

## Editing from the command line

`valle timeline edit` makes the same edits Studio does and writes the file back only if the
result is valid. `valle timeline query` finds clips and gives the paths edits take.

```sh
valle timeline query edit.timeline.json --at 4
valle timeline edit edit.timeline.json --op '{"op":"move","clip":"shot","start":3}'
valle timeline edit edit.timeline.json --op '{"op":"trim","clip":"shot","duration":4}'
valle timeline edit edit.timeline.json --op '{"op":"set","clip":"shot","field":"opacity","value":0.8}'
valle timeline edit edit.timeline.json --op '{"op":"delete","clip":"shot"}'
valle timeline edit edit.timeline.json --op '{"op":"move","clip":"shot","track":"tracks.visual[1]"}'
valle timeline edit edit.timeline.json --op '{"op":"set","track":"music","field":"enabled","value":false}'
valle timeline edit --help    # every op and its rules
```

The result lists changed clips with their start and end times in seconds, plus removed
transitions and clips deleted through anchors. With `--json`, `changes` contains `kind`
(`clip`, `transition` or `track`), `action` (`added`, `updated`, `removed`), and `before`/`after`
summaries (`null` for the absent side). Clip summaries include `id`, `path`, `start`, `end`, `kind` and
`anchor`; transitions name their `path`, `from`, `to` and `transition`. Changes to a clip's
fields are reported even when its time span stays the same. Studio uses the same result.

Editing the JSON by hand is just as good; run `check` afterwards.

## Errors

Errors name the place in the document as a JSON path, then what it takes:

```text
error: edit.timeline.json: tracks.visual[0].items[1].kind: a visual clip is solid, image, video, motion or timeline, not "movie"
error: edit.timeline.json: tracks.visual[0].clips[0]: no field "source"; it takes id, name, …, component, props, …
```

Fix the field it names and check again. `--json` gives the same message in `message`.
