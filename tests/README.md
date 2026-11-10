# Writing tests

Start with the behavior to verify, then choose the check that can detect it.
Use small scenes with a few related variants. Keep test inputs separate from
expected data; normal test runs must never regenerate their own expectations.
All commands below run from the repository root.

Web unit tests live in each package's `tests/` directory alongside `src/`, as
`*.test.ts`: `web/packages/engine/tests/` and `web/apps/studio/tests/`. Run them
with `bun test ./web` (or `bun test` from the repository root). Use `bun:test`
assertions against observable behavior; replace browser transports only where
the test does not need an actual browser. For example, test export frame
selection in `messages.test.ts`, worker reply handling in `connection.test.ts`,
and revision conflicts in Studio's `service.test.ts`.

`cargo xtask test` runs these unit tests even with `--skip gpu,browser`, so all
CI platforms exercise them. The Playwright player and Studio tests in
`web/tests/` remain the checks for real WebGPU rendering, browser media APIs
and UI interaction.

| What you are testing              | What to compare                                                           | Example to adapt                                                                                            |
| --------------------------------- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| CSS layout, text or compositing   | Explicit numerical expectations and reviewed browser images               | [Content box](cases/css/box-model/content-box/scene.motion.tsx), [text](cases/css/text/preserved-spaces/scene.motion.tsx) |
| SVG rendering                     | Reviewed browser images, plus viewport/layout assertions where applicable | [Viewports and dashes](cases/svg/viewbox-dashes/inline/scene.motion.tsx)                                           |
| Motion or a Valle-specific effect | Reviewed frame PNGs and seek consistency                                  | [Echo](cases/motion/echo/scene.motion.tsx), [Shutter](cases/motion/shutter/scene.motion.tsx)                              |
| D3-compatible calculations        | Recorded results from the official D3 package                             | [Function calls](cases/d3/cases.ts)                                                                         |
| Tailwind expansion                | Official CSS, class order and expected rejection                          | [Utility samples](cases/tailwind/classes.json)                                                              |
| Tailwind rendering                | Reviewed Valle PNGs after class expansion and painting                    | [Flex utilities](cases/tailwind/flex/scene.motion.tsx)                                                             |
| Timeline behavior                 | Reviewed frames around clip boundaries and seek consistency               | [Clips](cases/timeline/clips/scene.timeline.json)                                                           |

## CSS and SVG

1. Create `tests/cases/css/<feature>/<name>/scene.motion.tsx` or
   `tests/cases/svg/<feature>/inline/scene.motion.tsx`.
   Use `fps: 1, duration: 1` for a static test. Set the canvas size explicitly.
2. Add `case.json` beside it. State the rule in `assertion`, link its specification
   clause in `spec`, and sample the single frame with `at: [0]`.
3. Give measured elements stable JSX keys. For layout behavior, add independently
   derived `layout` expectations for border, padding and content boxes; for text
   wrapping, add `lines`. Pure color, filter or compositing tests can use pixels
   alone: an unchanged outer box does not verify those behaviors.
4. Write an equivalent browser page at
   `tests/baselines/browser/<case>/scene.html`, where `<case>` includes `css/` or
   `svg/`. Write this HTML independently, rather than using Valle's evaluated
   layout to construct the expected page.
5. Capture all three browsers, review their results against the specification,
   select the baseline, then run the Valle test.

For example, this is the essential setup of the content-box case:

```tsx
export const composition = { width: 200, height: 140, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 200, height: 140, backgroundColor: "white" }}>
      <View
        key="box"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          boxSizing: "content-box",
          width: 100,
          height: 60,
          padding: 10,
          border: "4px solid black",
          backgroundColor: "#00aa00",
        }}
      />
    </View>
  );
}
```

Its content box is 100×60. Padding adds 20px per dimension and borders add 8px,
so the border box is 128×88. Encode these expectations explicitly:

```json
{
  "spec": "https://www.w3.org/TR/css-sizing-3/#box-sizing",
  "assertion": "Content-box dimensions exclude padding and borders.",
  "at": [0],
  "browser": {
    "selected": "chrome",
    "reason": "The specification-derived box sizes and all three reviewed captures agree."
  },
  "layout": [
    {
      "key": "box",
      "border": { "x": 20, "y": 20, "width": 128, "height": 88 },
      "padding": { "x": 24, "y": 24, "width": 120, "height": 80 },
      "content": { "x": 34, "y": 34, "width": 100, "height": 60 }
    }
  ]
}
```

Use [the matching HTML](baselines/browser/css/box-model/content-box/scene.html)
as the browser-page template. The capture script requires:

- `#canvas` starts at `(0,0)` and has integer dimensions matching the TSX canvas.
- `data-key="box"` identifies the same element as JSX `key="box"`. For nested
  keys, use the full JSX key path, such as `container/item`.
- Browser defaults are explicit where they matter: body margin, box sizing,
  fonts, line height and background. Valle defaults to `border-box`; browsers
  normally default to `content-box`.
- Fonts and other shared assets are fixed local files under `tests/resources/`.
  Both browser pages and scene resources use `/tests/resources/...`.

For SVG, keep explicit dimensions and test a few related combinations together,
for example `viewBox` with `preserveAspectRatio`. The layout assertions measure
boxes; they do not assert the shape of a path. Use the image comparison for
fills, strokes, clipping and path geometry. WPT import and reftests are not
implemented in this runner yet.

Exercise both SVG input paths when adding a supported feature. Keep them together
as `svg/<feature>/inline/` and `svg/<feature>/file/`. The [inline gradients
case](cases/svg/gradients/inline/scene.motion.tsx) tests JSX attributes; its [file
counterpart](cases/svg/gradients/file/scene.motion.tsx) tests XML parsing and resource
loading through `Image`. Put that case's SVG beside its scene as `source.svg`,
referenced with `./source.svg`; moving the case then preserves the resource path.
The browser page loads the same SVG through `/tests/cases/svg/<feature>/file/source.svg`.
Each input path retains its own browser page and reviewed captures. Do not replace
either entry with a rendering generated by the other.

File-only behavior, such as [embedded images](cases/svg/embedded-image/file/scene.motion.tsx),
needs only the `file/` case. SVG `<text>` is not implemented; its rejection is tested
beside the SVG file parser. Shared fonts and images stay in `tests/resources/`;
single-case assets belong beside the case that owns them.

The test loader mounts the repository at `/`, matching the browser capture
server. Write shared resource defaults as `/tests/resources/photo.png`, not a
chain of `../` segments. Timeline resources and nested Motion components use the
same paths. This is the test host's file mapping, not a new engine URL scheme or
a machine's absolute directory. Local `./source.svg` still resolves beside its
scene. The relocation regression copies entries to two different directory
depths and checks their frames and decoded resources without rewriting paths.

The [viewBox file case](cases/svg/viewbox-dashes/file/scene.motion.tsx) uses `source.svg`
for normalized dashes and four adjacent SVG files for the different root
`preserveAspectRatio` values. It covers the same picture as its inline counterpart
without requiring nested SVG documents.

Put CSS combinations under the behavior they verify, such as `css/flex/grid-child`
or `css/compositing/backdrop-blend/overflow`. Keep a regression's diagnosis and
controls in a README at the feature level when needed; do not create a separate
collection for all combinations.

### Text and color checks

Use `lines` to assert the exact text of each line and its `{ x, baseline, height }`.
See [the text case configuration](cases/css/text/preserved-spaces/case.json).
Box values and line strings compare exactly; line geometry allows 2/64px.
The same numerical expectations apply at every sampled time.

On the browser page, mark the text element with both `data-key` and `data-text`.
The capture helper currently supports one text node with a uniform font and
explicit line height. Preserve intentional spaces and newlines, and use the
same pinned font in Valle and `@font-face`.

Put `<!-- prettier-ignore -->` before text whose whitespace must remain exact.
Use [word wrapping](cases/css/text/word-wrap/scene.motion.tsx) for normal break opportunities
and [break-all](cases/css/text/wrapping/scene.motion.tsx) for breaks inside a word.
The browser helper measures ranges on the unchanged paragraph and calibrates
the baseline using a separate font probe; inserting markers into the paragraph
can alter collapsed spaces. For mixed-font spans, use image checks and box
assertions, as in [mixed runs](cases/css/text/mixed-runs/scene.motion.tsx); do not mark
that paragraph `data-text`, which only measures uniform-font text.

Test colors in solid regions so antialiasing cannot obscure the result. Default
image comparisons are exact. If a difference is limited to rasterized edges,
inspect it before adding a case-specific allowance:

- `browser.pixels` has `maxDifference` (largest channel delta) and `maxPixels`
  (number of changed pixels). Both limits must pass.
- `browser.textEdges` can apply an allowance only to glyph edges. Its `key` must
  have both layout and line assertions. This option is for text on a flat
  background; it does not permit differences throughout the text box.
- Record why an allowance is justified in `browser.reason`. Do not copy another
  case's tolerance or increase it merely to make a failure pass.

### Capture and select browser baselines

```sh
bun install --frozen-lockfile
bun run playwright install chromium firefox webkit --no-shell
bun tests/tools/browser.ts
```

The script captures every case declaring `browser`, using Chromium, Firefox and
WebKit managed by the pinned Playwright package. Chromium uses the full browser's
headless mode, shared with the WebGPU player and Studio tests. Candidates go to
`target/browser-baselines/`; accepted files remain unchanged.
The script prepares the pinned fonts through `cargo xtask fonts` before opening
the pages, using the same definitions and download cache as the native tests.
To capture only changed cases, pass one or more case paths or category prefixes,
for example `bun tests/tools/browser.ts css/grid svg/gradient-spread/inline`.

Inspect the three PNGs and their JSON measurements against the cited rule.
Browser agreement helps, but the specification decides defined behavior.
Record the selected browser and reasoning in `case.json`. Copy the reviewed
PNG/JSON pairs into `tests/baselines/browser/<case>/`, keeping all three engines
for diagnostics. Keep the shared `environment.json` consistent with the accepted
captures; a browser-version change requires reviewing the affected baselines.

`cargo xtask test` installs Playwright's Chromium if needed before running the
player and Studio tests. Firefox and WebKit are only needed for manual baseline
capture. `--skip browser` leaves out browser installation and those two tests.

The JSON files record dimensions, measured boxes/lines and input hashes. Changes
to the HTML or font invalidate them. Recapture and review them instead of
editing hashes to suppress a stale-baseline error.

If a reviewed baseline exposes an engine bug, keep the failing case and record
the controls, measurements and code path in a short README beside it. Do not
accept the current Valle output or widen the allowance just to make the test
pass. See [the backdrop/blend regression](cases/css/compositing/backdrop-blend/README.md).

## Motion and Valle-specific effects

Create `tests/cases/motion/<name>/scene.motion.tsx` and `case.json`. Pick times that
exercise distinct states: before an effect starts, during it, at a branch or
loop boundary, and at the last valid frame. Short examples can cover every frame.
Use a fixed seed for randomness.

For example, a one-second scene at 4fps samples four frames:

```json
{
  "spec": "crates/valle-engine/src/front/runtime.js:interpolate",
  "assertion": "The square moves 16px between successive sampled frames.",
  "at": [0, 0.25, 0.5, 0.75],
  "valle": {
    "reason": "Record the verified positions and visual review here before accepting the PNGs.",
    "maxDifference": 0
  }
}
```

`at` contains seconds, in increasing order, inside the scene duration. Output
and baseline names use the actual zero-based frame number: at 30fps, sampling
seconds `[0, 1]` refers to frames `0` and `30`, not `0` and `1`. Use at most one
sample in each frame. Do not sample `duration` itself.

Keep the intended frame rate when it affects semantics. For example, Shutter's
exposure depends on fps; reducing it to speed up the test changes the effect.
Include useful controls, such as a stationary object or zero shutter angle.
For a mathematical helper, also write numerical unit tests beside its source;
a PNG regression alone does not prove its formula.

Multiple-time cases automatically compare forward, reverse and alternating
seeks with fresh scene instances. Frame data must agree; with GPU checks enabled,
pixels must agree exactly too. No extra configuration is needed.

### Accept a Valle image baseline

1. Run `cargo xtask cases motion/<name>`. A missing baseline fails the test but
   leaves `actual.png` for inspection.
2. Check every sampled image against the stated behavior and expected values.
   Record what was verified in `valle.reason`.
3. Copy reviewed images into `tests/baselines/valle/motion/<name>/<frame>.png`.
   A single-sample result is at `target/test-results/<case>/actual.png`;
   multiple samples are under `<case>/<frame>/actual.png`.
4. Rerun the case. Do not replace an existing baseline simply because it differs;
   establish whether the new result is an intended behavior change first.

`valle.maxDifference` bounds each channel's difference and defaults to zero.
Some existing product baselines allow one level for rounding; use that only
when justified by review. `valle.maxPixels`, when present, also bounds how many
pixels may differ, as `browser.pixels` does; use it when a few pixels differ
by more between GPUs, and give the measured counts in `valle.reason`. Product
baselines are reviewed regressions, not independent proofs of correctness.
There is no automatic accept command.

## D3 calculations and charts

Add a named function to [cases/d3/cases.ts](cases/d3/cases.ts). Use the supported
D3 globals so the same calls run through Valle and the official library:

```ts
export const cases = {
  midpoint: () => scaleLinear().domain([0, 10]).range([0, 200])(5),
};
```

Choose inputs that exercise normal values and relevant boundaries. Return
JSON-serializable results such as numbers, arrays, objects or path strings.
Then generate official candidates:

```sh
bun tests/tools/d3.ts
```

Review `target/baseline-candidates/d3/results.json`, including the expected
result names and values, before copying it to `tests/baselines/d3/results.json`.
The baseline records the pinned D3 version and source hash; changing the calls
requires recapture. Normal tests execute Valle's helpers and compare with the
saved official results, without launching Bun or needing a GPU.

Use `cargo xtask cases d3/cases` to run the function checks. To also test chart
painting, add a scene like [linear-path](cases/d3/linear-path/scene.motion.tsx) and use
reviewed Valle PNGs following the image-baseline steps above.

## Tailwind utilities and rendering

For utility parsing or expansion, add relevant syntax examples to
[cases/tailwind/classes.json](cases/tailwind/classes.json), then run:

```sh
bun tests/tools/tailwind.ts
```

The official compiler's candidate output is
`target/baseline-candidates/tailwind/compiled.json`. It includes listed utilities,
additional samples, CSS, class order and custom-property defaults. Review it
before updating `tests/baselines/tailwind/classes.json`:

- `expanded`: supported classes and their official CSS, in official order.
- `invalid`: samples for which the official compiler emits no CSS.
- `refused`: valid official classes deliberately unsupported by Valle.
- `properties`: official custom-property defaults.

Do not copy `compiled.json` directly: its raw output does not contain the reviewed
support categories. Do not derive expected CSS from Valle's own expansion.
Run `cargo xtask cases tailwind/classes` to check expansion, ordering, defaults
and rejection without rendering a scene.

For layout or painting after expansion, add
`tests/cases/tailwind/<name>/scene.motion.tsx` using `className`, with a matching
`case.json` and reviewed Valle PNGs. Follow the normal image-baseline workflow.
The existing [flex](cases/tailwind/flex/scene.motion.tsx) and
[text](cases/tailwind/text/scene.motion.tsx) cases show how to group related variants.
Keep the PNG regression and add numeric expectations when classes control box
sizes, placement or line breaking. Use the 16px root size and 4px spacing unit
for arithmetic; verify font-dependent expectations independently with the pinned
font and official Tailwind CSS. Do not copy the engine's current measurements
into expected data.
Run `cargo xtask cases tailwind` for both utility and rendering checks.

## Timeline

Subtitle behavior is tested with Rust assertions beside the Timeline lowering code in
`crates/valle-engine/src/timeline.rs`: visible glyphs and colors at run boundaries,
opacity and transforms, exact line strings, text bounds, stable glyph positions,
presenter props and local clocks, overlapping tracks, and joined emoji. These tests
need no GPU or stored PNGs:

```sh
cargo test -p valle-engine --lib timeline::tests
```

`resources/timeline/captions/` contains shared inputs for those assertions and the
browser checks in `web/tests/player.ts`. The browser compares a few frames with
fresh native renders, then exports and reimports a subtitle video. Generated images
are diagnostics, not permanent subtitle baselines; do not save a PNG for each timing
assertion. CSS/text rendering has its own tests.

Create `tests/cases/timeline/<name>/scene.timeline.json` using the normal Timeline
format. Adapt [clips](cases/timeline/clips/scene.timeline.json) for placement and
time mapping, or [nested](cases/timeline/nested/scene.timeline.json) for nesting.
Set `input` in the accompanying `case.json`:

```json
{
  "input": "scene.timeline.json",
  "spec": "crates/valle-engine/src/timeline.rs",
  "assertion": "Describe the clip timing, composition or transition being verified.",
  "at": [0, 0.25, 0.5, 0.75],
  "valle": {
    "reason": "Record the frame-by-frame review before accepting the PNGs.",
    "maxDifference": 0
  }
}
```

Match sampling times to the timeline's fps and duration. Include frames on both
sides of relevant clip boundaries, gaps, transitions, trims, loops or holds.
Shared Motion components and media go in `tests/resources/`, referenced as
`/tests/resources/...`. This also applies inside shared components and nested
Timelines. Use `./...` for resources owned by the case itself.

The runner uses the production Timeline-to-scene conversion, then the same frame,
image and seek checks as Motion. Cases with videos need system FFmpeg 7, 8 or 9;
the test loader explicitly uses software decoding. Accept reviewed PNGs under
`tests/baselines/valle/timeline/<name>/<frame>.png` and run
`cargo xtask cases timeline/<name>`.

## Run and investigate

```sh
cargo xtask cases                         # All scene and dependency cases, with GPU rendering
cargo xtask cases css/box-model/content-box
cargo xtask cases timeline                # One category
cargo xtask test --skip gpu,browser       # CPU checks across the workspace
```

A scene's `input` defaults to `scene.motion.tsx`. Each `case.json` needs `spec`,
`assertion`, `at`, and at least one numerical assertion or image baseline.
Keep source code in TSX/HTML, not JSON strings. Keep shared assets local and
versioned under `tests/resources/`, including their licenses where applicable.
The two built-in test fonts are generated exceptions: scene tests prepare them
from the pinned download cache, and Git ignores the copied files. A cold cache
needs network access; download or verification errors fail the tests.

Read `target/test-results/<case>/result.json` for measured values and failures.
GPU runs also save the actual image, the selected baseline and difference images.
The console lists each case's checks (`boxes`, `lines`, `pixels`, `seek`) and the
number of numerical assertions and sampled frames. These are derived from the
case contents, not a second declaration of its test type.
White in a diff means equal; magenta marks changed pixels, and amber marks
changes inside a declared text-edge mask. Inspect the pictures as well as the
counts before deciding whether a failure is geometry, color or rasterization.
Only the selected browser decides browser comparison success; the other two
remain diagnostic. Without GPU checks, visual cases are marked `PARTIAL` in the
result data, and old image outputs are removed rather than shown as current.

For input rejection and algorithm edge cases, add unit tests in the corresponding
Rust source module. `case.json` currently describes successful scene samples;
it does not have an error-expectation field. The harness's own negative checks
live beside its Rust implementation and run with:

```sh
cargo test --locked -p valle-engine --test cases
```

## Fuzzing input parsers

`fuzz/` is a separate [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html)
package, so nightly and libFuzzer are not dependencies of ordinary builds. Its four
targets call the production parsers without a GPU, fonts, network or file imports:

- `css`: a camelCase property on the first line, its CSS value on the remaining
  lines; parses and computes the style of a View.
- `svg`: an SVG file, including its styles, references and embedded images.
- `timeline`: a Timeline JSON document and its validation.
- `tsx`: Motion source rewriting, declarations and import resolution. It does not
  execute the script or test frame evaluation.

Install the tools once, then run from the repository root:

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

On macOS, use Homebrew LLVM for the C/C++ dependencies too: Apple's Clang uses a
different AddressSanitizer ABI from Rust's runtime.

```sh
brew install llvm
export CC="$(brew --prefix llvm)/bin/clang"
export CXX="$(brew --prefix llvm)/bin/clang++"
export MACOSX_DEPLOYMENT_TARGET=11.0
```

Then run a bounded session:

```sh
mkdir -p fuzz/corpus/css
cargo +nightly fuzz run css fuzz/corpus/css fuzz/seeds/css -- \
  -max_total_time=60 -max_len=16384 -timeout=10 -rss_limit_mb=2048
```

Replace `css` with `svg`, `timeline` or `tsx`. These limits bound this fuzz run,
not what Valle accepts. `seeds/` contains small authored inputs; libFuzzer writes
new coverage inputs to the first corpus directory, which is ignored. Crashes,
timeouts and memory-limit failures go in `fuzz/artifacts/<target>/`.

A parser returning an error is normal; a panic or sanitizer failure is not. Replay
a failure with `cargo +nightly fuzz run <target> <artifact>`, reduce it with
`cargo +nightly fuzz tmin <target> <artifact>`, and turn the smallest reproducer
into a Rust regression test beside the affected code before fixing it. Those
regressions run in ordinary CI. Random fuzzing is a manual check; it neither
accepts image baselines nor proves a successful parse has the right semantics.
