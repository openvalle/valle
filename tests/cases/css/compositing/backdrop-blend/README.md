# Backdrop filter and blend regression

Run `cargo xtask cases css/compositing`. The original backdrop/blend case and
six additional controls cover opacity, masking, ancestor clipping, nested
stacking contexts, zero-sized filter regions and 3D planes. Browser PNGs were
retained; they were not replaced with Valle output.

The original four boxes use `backdrop-filter: invert(1)` and
`mix-blend-mode: multiply`: full opacity, half opacity, zero opacity, and an empty
transparent box. The page is RGB `(64, 128, 192)`. The separate
[controls](controls/scene.tsx) exercise invert and multiply individually.

## Observed results

Captured on macOS arm64, 2026-11-06; browser versions are in the shared
`tests/baselines/browser/environment.json`. Coordinates are image pixels.

| Sample                           | Chrome      | Valle before | Valle after |
| -------------------------------- | ----------- | ------------ | ----------- |
| Full-opacity background (30,30)  | 191,95,31   | 56,64,24     | 191,95,32   |
| Full-opacity child (70,70)       | 0,127,32    | 0,128,96     | 0,127,32    |
| Half-opacity background (190,30) | 128,112,112 | 60,96,108    | 128,112,112 |
| Empty filtered box (510,30)      | 191,127,63  | 48,64,47     | 191,127,63  |

Before the fix, 58,800 pixels differed from Chrome, with a maximum channel
difference of 143. These were solid regions, not antialiasing edges. The empty
box exposed the error directly: inversion should give `(191,127,63)`, but
multiplying that result by the original page again gives about `(48,64,47)`.

After the fix, only the full-opacity translucent background differs: blue is
32 instead of 31 across 16,000 pixels. The unquantized result is 31.5; this is a
one-level output rounding difference, not the previous blending error. The
case permits at most one channel level and 16,000 affected pixels. Removing
multiply or inversion still fails, as does moving the image by one pixel.

Native PNGs, difference images and measurements are written to
`target/test-results/css/compositing/backdrop-blend/basic/` on each GPU test run.
All three browser captures are retained in the matching baseline folder.

## What changed

In `crates/valle-engine/src/paint.rs`, `over_backdrop()` previously grouped the
foreground with `Blend::Normal`, then `layer()` and `planes()` blended the
foreground **and filtered backdrop** with the original page. It now blends the
foreground with the filtered backdrop, returns `Blend::Normal` for the outer
composite, and leaves opacity, masks, clips and transforms around that result.
Half-opacity and zero-opacity controls verify opacity applies to the combined
result once.

Two additional probes caught related isolation errors:

- A zero-width filter region emits no backdrop pixels, but must still isolate
  its overflowing foreground. Returning the original outer blend on this path
  incorrectly multiplied the overflow by the page.
- `scene()` projected and sorted 3D planes directly into the parent, bypassing
  the isolation applied to ordinary stacking contexts. A scene containing
  blending descendants now groups its projected planes before compositing to
  its parent. Plane sorting and clipping are unchanged.

[Compositing 1 section 3.2](https://www.w3.org/TR/compositing-1/#csscompositingrules)
requires stacking contexts to be isolated;
[CSS Transforms 2 section 7](https://www.w3.org/TR/css-transforms-2/#transform-style-property)
makes `preserve-3d` establish a stacking context. The separate scene probe uses
only solid colors and multiply, without backdrop filtering, and matches Chrome
and WebKit exactly: cyan remains cyan over a transparent scene, cyan over the
scene's yellow background becomes green, and cyan/magenta planes overlap in blue.

## Additional image checks

Each case retains all three browser captures. The selected Chrome differences
below were checked in the actual images and difference maps. Nonzero allowances
are limited to one channel level in translucent regions; they do not permit
missing effects, displaced geometry or large color errors.

| Case suffix                | Checks                                            | Changed pixels / maximum difference |
| -------------------------- | ------------------------------------------------- | ----------------------------------- |
| `clips`                    | Clip path and hard mask, full/half/zero opacity   | 4,400 / 1                           |
| `nested`                   | Isolated colored parent, parent and child opacity | 14,400 / 1                          |
| `planes`                   | 2D control, isolated 3D plane, half-opacity plane | 33,600 / 1                          |
| `overflow`                 | Visible overflow versus ancestor clipping         | 15,100 / 1                          |
| `empty-bounds`             | Zero-width filter versus multiply-only control    | 0 / 0                               |
| `../scene-blend-isolation` | Scene-local background and overlapping planes     | 0 / 0                               |

The inline paint tests also verify the layer structure for 2D, 3D and zero-width
filter regions. These geometric probes do not establish correctness for every
blur kernel, blend mode, backdrop-root interaction or perspective transform.

## Browser capture limitations

In the current Playwright runs, Firefox and WebKit omit backdrop inversion
**even without blending, opacity or a mask**. A separate empty-box probe and the
invert-only control both show this. All three engines report parsing support
and computed `backdropFilter === "invert(1)"`, but only Chrome paints inversion.
The precise capture/backend cause is undetermined; this does not prove their
interactive browsers implement the effect incorrectly. The current Firefox
capture also omits the translated 3D planes. WebKit matches Chrome in the
standalone 3D isolation probe, but differs on the zero-width multiply-only box.

The selected backdrop target follows the source-over structure in
[Filter Effects 2 section 2.1](https://drafts.csswg.org/filter-effects-2/#BackdropFilterProperty).
Its Backdrop Root definition is still an Editor's Draft without Working Group
consensus. These baselines record a reasoned interoperability target, not proof
that all backdrop interactions are settled. Do not choose a baseline merely
because two browser screenshots agree.
