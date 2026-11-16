# CSS support and test coverage

This is the repository's CSS capability and coverage inventory. Use it to choose
authoring syntax and to find missing tests. [Motion](motion.md#styles) explains how
to write scenes; [Writing tests](../tests/README.md) explains how to add evidence.

Property inventory audited against `cd645114`; combination coverage expanded on
2026-11-16. The CSS suite contains **177 scenes** with **128,662 explicit
layout/line assertions** across their sampled frames. Validation results and
platform boundaries are recorded below. This is not a claim of full CSS
conformance.

- The [property table](../crates/valle-engine/src/style/property.rs) has 286 names:
  **284 CSS/SVG style names**, plus Valle's `motionBlur` and XML's `xmlSpace`.
  Shorthands, logical properties and aliases count separately. All 284 names are
  listed below; a group of names does not imply identical applicability.
- There are **157 static browser cases and 20 dynamic cases**. Layout expectations
  cover 10,676 elements in 111 fixtures; line expectations cover 46 text objects
  in 10 fixtures. These groups overlap. Of the boxes, 9,742 belong to the bounded
  fixed-seed generator, so the larger count does not imply proportionally broader
  CSS coverage.
- Parser/computed-style unit tests, SVG cases, Tailwind cases and dynamic Motion
  cases provide additional evidence. None makes an untested value or combination
  automatically covered.

## Reading the inventory

The property tables describe implemented syntax and important restrictions.
The evidence column points to tests with relevant assertions, not just files
where a property happens to occur. Follow a case's `assertion`, `layout`, `lines`
and selected baseline to see exactly what it verifies.

| Evidence              | What it establishes                                                             | What it does not establish                                         |
| --------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| CSS or SVG case       | A reviewed browser image and any explicit numerical expectations for that scene | Every value, all combinations, or correctness on every platform    |
| Unit test             | Its named parsing, calculation, error or geometry assertions                    | The final rendered appearance unless pixels are explicitly checked |
| Tailwind case         | Reviewed Valle output after class expansion; some cases also assert geometry    | An independent browser oracle for every CSS effect in the picture  |
| Native/Web comparison | The selected scenes agree across the two runtime paths                          | Correct CSS semantics: both paths can share a bug                  |
| Fresh/seek comparison | A sample does not depend on the tested visit order                              | Whether that sample agrees with the specification                  |
| Not mapped            | This audit has not established a targeted test for the stated behavior          | That the implementation is absent, or that no indirect test exists |

Rows sharing tests still need separate value/boundary checks. The
[combination backlog](#combination-coverage-and-gaps) records missing coverage;
there is no measured global pairwise or three-way coverage percentage yet.

## Common values and cascade

Implementation: [units](../crates/valle-engine/src/style/units.rs),
[math](../crates/valle-engine/src/style/math.rs),
[color](../crates/valle-engine/src/style/color.rs),
[cascade](../crates/valle-engine/src/style/cascade.rs).

| Input                  | Implemented scope and restrictions                                                                                                                                                                                                                                                                                                 | Existing evidence                                                                                                         |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Property spelling      | React-style camelCase in TSX; CSS names are converted by the stylesheet/Tailwind paths. Unknown properties and unsupported values produce errors.                                                                                                                                                                                  | `property.rs`; stylesheet and Tailwind tests                                                                              |
| Numeric lengths        | A JS number is px for a length property. A nonzero length in a CSS string needs a unit; SVG lengths additionally accept unitless numeric strings. Negative values and percentages depend on the property.                                                                                                                          | `units.rs` tests; [absolute units][css/values/absolute-units]                                                             |
| Length units           | `px`, `cm`, `mm`, `in`, `pt`, `pc`, `em`, `ex`, `rem`, `%`, `vw`, `vh`, `vmin`, `vmax`, `vi`, `vb`, and small/large/dynamic viewport variants. `rem` uses a fixed 16px root; all viewport variants use the scene canvas. `vi`/`vb` use the canvas's horizontal/vertical axes. No `ch`, `lh`, container units or device-DPI sizing. | `viewport_units_take_the_canvas`, `absolute_units_use_css_reference_pixels_in_lengths_and_math`; [ex][css/values/font-ex] |
| Math                   | `calc()`, `min()`, `max()`, `clamp()` with nested arithmetic and compatible units; percentage terms resolve against the eventual reference size. Not the entire CSS math-function family.                                                                                                                                          | `math.rs` tests; [percentage constraints][css/sizing/percentage-min-max]                                                  |
| Colors                 | Named/hex colors, rgb/rgba, hsl/hsla, hwb, lab/lch, oklab/oklch, supported `color()` spaces, relative colors and `color-mix()`. `currentColor` resolves at the consuming element, including nested mixes. Parsing a wide-gamut color does not select a video output gamut.                                                         | `color.rs` tests; [sRGB/alpha][css/color/srgb-alpha], [currentColor mixes][css/color/current-mix]                         |
| CSS-wide keywords      | `inherit`, `initial`, `unset`, including shorthand expansion and declaration order. Not `revert`, `revert-layer` or the `all` shorthand. Valle's element defaults can differ from CSS initial values, notably border-box sizing.                                                                                                   | `every_css_property_accepts_all_three_keywords`; [wide keywords][css/cascade/wide-keywords]                               |
| Class/style precedence | Tailwind expansion feeds the style layer. Inline style wins over normal classes; important classes can override it. CSS custom properties supplied by authors are not supported.                                                                                                                                                   | [Tailwind spacing](../tests/cases/tailwind/spacing/case.json), official conversion suite                                  |

The `color()` RGB/XYZ spaces are srgb, srgb-linear, display-p3, a98-rgb,
prophoto-rgb, rec2020, xyz/xyz-d65 and xyz-d50. The color-mix parser also supports
lab, oklab, hsl, hwb, lch and oklch with the implemented hue interpolation modes.
These are accepted input forms. The [color-space/opacity matrix][css/color/spaces-opacity]
adds 24 browser controls and a dynamic companion to the sRGB/alpha and
currentColor cases; numeric tests cover additional conversion boundaries.

Coverage: [inherited units][css/values/inherited-units] crosses px/em/rem/% with
three writing modes and parent font sizes; its dynamic companion changes the
inherited basis. [Logical order][css/cascade/logical-order] crosses both directions
and both declaration orders. These are bounded value matrices, not every unit
at every consuming property.

## Boxes, positioning and overflow

Implementation: [declarations](../crates/valle-engine/src/style/declaration.rs),
[computed styles](../crates/valle-engine/src/style/computed.rs),
[layout](../crates/valle-engine/src/layout.rs),
[inline boxes](../crates/valle-engine/src/layout/atoms.rs).

| Properties                                                                                                        | Implemented scope and restrictions                                                                                                                                                                                                                                              | Existing evidence                                                                                                                                                    |
| ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `display`                                                                                                         | `block`, `flex`, `grid`, `none`, `inline-block`, `inline-flex`, `inline-grid`, context-dependent `inline`, and `-webkit-box` for line clamp. Inline Views belong in Text/Span; flex/grid children blockify. No table/list-item/contents/flow-root modes or arbitrary HTML flow. | [block flow][css/layout/block-flow], [inline boxes][css/text/inline-boxes], `inline_displays_blockify_in_flex_and_grid_containers`                                   |
| `position`                                                                                                        | `static`, `relative`, `absolute`, `fixed`; no sticky. Out-of-flow boxes directly in a paragraph are rejected.                                                                                                                                                                   | [absolute/relative][css/position/absolute-relative], [containing block][css/position/absolute-padding]; fixed containing-block unit tests                       |
| `inset`, `top`, `right`, `bottom`, `left`                                                                         | Signed length/percentage or `auto`; inset takes 1–4 values. Positioning uses the applicable containing block.                                                                                                                                                                   | [absolute padding][css/position/absolute-padding], [vertical positioning][css/layout/vertical-position]                                                              |
| `insetInline`, `insetBlock`, `insetInlineStart`, `insetInlineEnd`, `insetBlockStart`, `insetBlockEnd`             | Logical counterparts; pair shorthands take 1–2 values and resolve with writing mode/direction.                                                                                                                                                                                  | [vertical positioning][css/layout/vertical-position], [logical sides][css/layout/logical-sides-matrix] |
| `width`, `height`, `inlineSize`, `blockSize`                                                                      | Nonnegative length/percentage, `auto`, `min-content`, `max-content`, `fit-content`, `stretch`. `fit-content()` is only accepted for grid tracks. Span text runs are not independent sized boxes.                                                                                | [intrinsic width][css/sizing/intrinsic-width], [vertical block][css/layout/vertical-block], [inline writing][css/text/inline-writing]; `sizing_keywords_go_to_taffy`, [logical sides][css/layout/logical-sides-matrix] |
| `minWidth`, `minHeight`, `minInlineSize`, `minBlockSize`                                                          | Nonnegative length/percentage or `auto`. Intrinsic sizing keywords are not accepted here.                                                                                                                                                                                       | [min/max][css/sizing/percentage-min-max], [height limits][css/sizing/auto-height-limits], `flow_relative_sizes_overflow_and_corners_follow_the_writing_mode` |
| `maxWidth`, `maxHeight`, `maxInlineSize`, `maxBlockSize`                                                          | Nonnegative length/percentage or `none`. Intrinsic sizing keywords are not accepted here.                                                                                                                                                                                       | [min/max][css/sizing/percentage-min-max], [height limits][css/sizing/auto-height-limits]                                                                             |
| `boxSizing`                                                                                                       | `border-box`, `content-box`; Valle defaults to border-box.                                                                                                                                                                                                                      | [border box][css/box-model/border-box], [content box][css/box-model/content-box]                                                                                     |
| `aspectRatio`                                                                                                     | `auto`, nonnegative ratio, or auto plus ratio. A degenerate ratio behaves as auto.                                                                                                                                                                                              | [aspect ratio][css/sizing/aspect-ratio], [vertical grid][css/layout/vertical-grid]; `box_sizing_aspect_ratio_and_order`                                              |
| `margin`, `marginTop`, `marginRight`, `marginBottom`, `marginLeft`                                                | Signed length/percentage or `auto`; 1–4 values in the shorthand. Margins do not apply to a Span text run.                                                                                                                                                                       | [block centering][css/layout/block-flow], [reverse/auto margins][css/flex/reverse-auto-margins], [Tailwind spacing](../tests/cases/tailwind/spacing/case.json)       |
| `marginInline`, `marginBlock`, `marginInlineStart`, `marginInlineEnd`, `marginBlockStart`, `marginBlockEnd`       | Logical signed lengths/percentages/auto; pair shorthands take 1–2 values.                                                                                                                                                                                                       | [bidi][css/text/bidi], [vertical block][css/layout/vertical-block], [logical sides][css/layout/logical-sides-matrix] |
| `padding`, `paddingTop`, `paddingRight`, `paddingBottom`, `paddingLeft`                                           | Nonnegative length/percentage; no auto. Padding and borders on Span participate in inline wrapping.                                                                                                                                                                             | [percentage padding][css/box-model/percentage-padding], [inline edges][css/text/inline-box]                                                                          |
| `paddingInline`, `paddingBlock`, `paddingInlineStart`, `paddingInlineEnd`, `paddingBlockStart`, `paddingBlockEnd` | Logical nonnegative lengths/percentages; pair shorthands take 1–2 values.                                                                                                                                                                                                       | [vertical container][css/text/vertical-container], [vertical positioning][css/layout/vertical-position], [logical sides][css/layout/logical-sides-matrix] |
| `overflow`, `overflowX`, `overflowY`, `overflowInline`, `overflowBlock`                                           | `visible`, `hidden`, `clip`; overflow takes one or two values. No scrolling or `auto`/`scroll`.                                                                                                                                                                                 | [clipping][css/overflow/clipping], [rounded child][css/shadow/rounded-clip]; `overflow_hidden_on_one_axis_hides_both`, `flow_relative_sizes_overflow_and_corners_follow_the_writing_mode` |
| `order`                                                                                                           | Signed integer; affects flex/grid item order rather than source text order.                                                                                                                                                                                                     | [alignment/order][css/flex/alignment-order], [Tailwind flex](../tests/cases/tailwind/flex/case.json)                                                                 |
| `zIndex`                                                                                                          | `auto` or signed integer, subject to stacking-context boundaries.                                                                                                                                                                                                               | [stacking/opacity][css/compositing/stacking-opacity]; `z_index_is_auto_or_an_integer`                                                                                |
| `visibility`                                                                                                      | `visible`, `hidden`, `collapse`; collapse is implemented as hidden, not flex/table collapse layout.                                                                                                                                                                             | [Tailwind sizing](../tests/cases/tailwind/sizing/case.json); collapse-specific geometry not mapped                                                                   |

Coverage: [indefinite percentages][css/sizing/indefinite-percentages] crosses
block/Flex/Grid with auto/definite heights and min/max constraints;
[intrinsic constraints][css/sizing/intrinsic-constraints] crosses the three size
keywords with two limits and both axes. Existing logical containing-block and
visibility controls remain complementary; fixed descendants of transformed
ancestors still have unit-level rather than independent rendered evidence.

## Flex, Grid and alignment

Implementation: [grid](../crates/valle-engine/src/style/grid.rs),
[declarations](../crates/valle-engine/src/style/declaration.rs), and the layout
adapter to Taffy. Unit tests include `the_flex_shorthand_follows_css`,
`grid_tracks_and_lines_go_to_taffy`, `grid_areas_and_shorthands_go_to_taffy`, and
`grid_values_out_of_css_are_errors`.

| Properties                                                                                            | Implemented scope and restrictions                                                                                                                                                                               | Existing evidence                                                                                                                       |
| ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `flexDirection`                                                                                       | `row`, `column`, `row-reverse`, `column-reverse`, mapped through writing mode.                                                                                                                                   | [vertical flex][css/layout/vertical-flex], [reverse][css/flex/reverse-auto-margins]                                                     |
| `flexWrap`                                                                                            | `nowrap`, `wrap`, `wrap-reverse`.                                                                                                                                                                                | [wrap/gap][css/flex/wrap-gap], [vertical flex][css/layout/vertical-flex]                                                                |
| `flexGrow`, `flexShrink`                                                                              | Nonnegative factors; constraints and automatic minimums affect distribution.                                                                                                                                     | [grow/shrink][css/flex/grow-shrink], [constrained growth][css/flex/constrained-growth], [automatic minimum][css/flex/automatic-minimum] |
| `flexBasis`                                                                                           | Nonnegative length/percentage, `auto`, `content`, `min-content`, `max-content`, `fit-content`, `stretch`.                                                                                                        | [percentage basis][css/flex/percentage-basis], [column minimum][css/flex/column-minimum]; shorthand unit tests                          |
| `flex`                                                                                                | `none`, `auto`, `initial`, or supported grow/shrink/basis forms; a lone factor uses a 0% basis.                                                                                                                  | `the_flex_shorthand_follows_css`; [Tailwind flex](../tests/cases/tailwind/flex/case.json)                                               |
| `justifyContent`, `alignContent`                                                                      | `normal`, `stretch`, `center`, `start`, `end`, `flex-start`, `flex-end`, `space-between`, `space-around`, `space-evenly`; safe/unsafe for supported positional values. No first/last-baseline content alignment. | [alignment][css/flex/alignment-order], [wrap/gap][css/flex/wrap-gap]; exhaustive safety/overflow matrix not mapped                      |
| `alignItems`, `justifyItems`                                                                          | `normal`, `stretch`, `center`, `start`, `end`, `flex-start`, `flex-end`, `self-start`, `self-end`, `baseline`; safe/unsafe positional alignment. No explicit first/last-baseline syntax.                         | [alignment][css/flex/alignment-order], [Tailwind grid](../tests/cases/tailwind/grid/case.json)                                          |
| `alignSelf`, `justifySelf`                                                                            | Item-alignment values above, plus `auto`. Applies where the layout model uses self alignment.                                                                                                                    | [alignment][css/flex/alignment-order], [Tailwind grid](../tests/cases/tailwind/grid/case.json)                                          |
| `placeItems`, `placeSelf`, `placeContent`                                                             | One or two values from the corresponding align/justify longhands.                                                                                                                                                | [Tailwind grid](../tests/cases/tailwind/grid/case.json); individual CSS shorthand combinations not mapped                               |
| `gap`, `rowGap`, `columnGap`                                                                          | Nonnegative length/percentage or `normal` (zero); gap takes one or two values.                                                                                                                                   | [wrap/gap][css/flex/wrap-gap], [grid tracks][css/grid/tracks], [vertical grid][css/layout/vertical-grid]                                |
| `gridTemplateRows`, `gridTemplateColumns`                                                             | `none`, fixed/percentage/fr tracks, auto/min/max-content, minmax, fit-content(), repeat with counts or auto-fill/auto-fit, and named lines. No subgrid or masonry.                                               | [tracks][css/grid/tracks], [auto repeat][css/grid/auto-repeat], [named lines][css/grid/named-lines-implicit]                            |
| `gridTemplateAreas`                                                                                   | `none` or rectangular named areas with empty cells.                                                                                                                                                              | [named areas][css/grid/named-areas]; `named_areas_are_case_sensitive_rectangles_with_empty_cells`                                       |
| `gridTemplate`, `grid`                                                                                | Explicit template/area shorthand; grid also supports the implemented auto-flow forms and resets its longhands.                                                                                                   | [named areas][css/grid/named-areas]; grid shorthand unit tests                                                                          |
| `gridAutoRows`, `gridAutoColumns`, `gridAutoFlow`                                                     | Implicit track lists; row/column with optional dense placement.                                                                                                                                                  | [dense placement][css/grid/auto-placement-dense], [implicit lines][css/grid/named-lines-implicit]                                       |
| `gridRow`, `gridColumn`, `gridRowStart`, `gridRowEnd`, `gridColumnStart`, `gridColumnEnd`, `gridArea` | Auto, signed line numbers, named lines/areas and span forms; row/column/area shorthands.                                                                                                                         | [placement][css/grid/placement], [named lines][css/grid/named-lines-implicit], [named areas][css/grid/named-areas]                      |

Coverage: the retained constraint/wrap matrices below and
[1,000 generated trees](../tests/cases/css/generated/README.md) combine nested
block/Flex/Grid, direction, alignment, gaps and box models. The generator omits
intrinsic text, wrapping and flex growth; the hand-authored matrices cover those
separately. No global pairwise coverage metric is claimed.

## Backgrounds and images

Implementation: [background](../crates/valle-engine/src/style/background.rs),
[gradient](../crates/valle-engine/src/style/gradient.rs),
[position](../crates/valle-engine/src/style/position.rs).
Unit tests in `declaration.rs` cover background lists, sizes, positions, repeats,
origins, clips and blend modes; `gradient.rs` tests stop/hint handling and errors.

| Properties                                                         | Implemented scope and restrictions                                                                                                                                                                 | Existing evidence                                                                                                                                                                        |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `background`                                                       | Layered shorthand with image, position/size, repeat, origin/clip and final color. It leaves backgroundBlendMode unchanged. No attachment/scroll behavior.                                          | [origin/clip][css/background/origin-clip]; `shorthand_keeps_nested_commas_and_sizes_attached_to_their_layers`                                                                            |
| `backgroundColor`                                                  | Color or currentColor, behind background image layers.                                                                                                                                             | [sRGB/alpha][css/color/srgb-alpha], [current mixes][css/color/current-mix]                                                                                                               |
| `backgroundImage`                                                  | Comma-separated none, resource images, linear/radial/conic gradients and repeating variants. Image URLs use declared `resource://` images, not arbitrary browser loading. No image-set/cross-fade. | [gradients][css/background/gradients], [SVG images][css/backgrounds/svg-image], [Tailwind backgrounds](../tests/cases/tailwind/backgrounds/case.json)                                    |
| `backgroundSize`                                                   | Per-layer contain, cover, or one/two nonnegative lengths/percentages/auto.                                                                                                                         | [SVG images][css/backgrounds/svg-image], [origin/clip][css/background/origin-clip]; background unit tests                                                                                |
| `backgroundPosition`, `backgroundPositionX`, `backgroundPositionY` | Per-layer positions, signed offsets, percentages and edge keywords.                                                                                                                                | [origin/clip][css/background/origin-clip]; axis/edge parser tests; rendered longhand ordering not mapped                                                                                 |
| `backgroundRepeat`                                                 | Repeat/no-repeat/space/round, repeat-x/repeat-y, per-axis and per-layer values.                                                                                                                    | Background unit tests, [SVG images][css/backgrounds/svg-image], [Tailwind backgrounds](../tests/cases/tailwind/backgrounds/case.json); independent round/space edge cases need expansion |
| `backgroundOrigin`, `backgroundClip`                               | Border/padding/content box; clip additionally supports text.                                                                                                                                       | [origin/clip][css/background/origin-clip], [inline directions][css/text/inline-directions], [Tailwind backgrounds](../tests/cases/tailwind/backgrounds/case.json)                        |
| `backgroundBlendMode`                                              | Comma-separated blend modes from the blend set below, within background layers.                                                                                                                    | `background_layers_take_blend_modes_which_the_shorthand_leaves`; [Tailwind blending](../tests/cases/tailwind/blending/case.json)                                                         |
| `objectFit`                                                        | `fill`, `contain`, `cover`, `none`, `scale-down` for replaced content.                                                                                                                             | [object fit][css/images/object-fit] explicitly checks fill/contain/cover; none/scale-down browser controls not mapped                                                                    |
| `objectPosition`                                                   | Position keywords and signed lengths/percentages within the replaced-content box.                                                                                                                  | [object position][css/images/object-position] checks left/center/right; remaining edge forms need controls                                                                               |
| `imageRendering`                                                   | `auto`, `pixelated`; no crisp-edges.                                                                                                                                                               | [object fit][css/images/object-fit], [embedded SVG images](../tests/cases/svg/embedded-image/file/case.json)                                                                             |

Coverage: [layer lists][css/background/layer-lists],
[fractional tiling][css/background/fractional-repeat],
[rounded positioning][css/background/rounded-position],
[position order][css/background/position-order],
[text effects][css/background/text-effects] and
[image fit][css/images/fit-extremes] retain independent browser controls.
Dynamic cases exercise list replacement, changed image constraints and tile sizes.

## Borders, corners, outlines and shadows

Implementation: [declarations](../crates/valle-engine/src/style/declaration.rs),
[border painting](../crates/valle-engine/src/paint/borders.rs),
[dash geometry](../crates/valle-engine/src/paint/dash.rs).
Unit tests include `borders_take_lines_widths_styles_and_colors`,
`corner_radii_expand_as_sides_do`, `outlines_take_a_line_and_an_offset`, and
`box_shadows_take_lengths_a_color_and_inset`.

| Properties                                                                                                                                                                                                                             | Implemented scope and restrictions                                                                                                          | Existing evidence                                                                                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `border`, `borderTop`, `borderRight`, `borderBottom`, `borderLeft`                                                                                                                                                                     | Width/style/color shorthand, omitted parts reset.                                                                                           | [styles/radii][css/border/styles-radius]; border unit tests                                                                                       |
| `borderInline`, `borderBlock`, `borderInlineStart`, `borderInlineEnd`, `borderBlockStart`, `borderBlockEnd`                                                                                                                            | Logical edge/pair line shorthands.                                                                                                          | [bidi][css/text/bidi], [Tailwind borders](../tests/cases/tailwind/borders/case.json); every shorthand in all directions not mapped, [logical sides][css/layout/logical-sides-matrix]; `flow_relative_and_physical_border_declarations_cascade_in_written_order` |
| `borderWidth`, `borderTopWidth`, `borderRightWidth`, `borderBottomWidth`, `borderLeftWidth`, `borderInlineWidth`, `borderBlockWidth`, `borderInlineStartWidth`, `borderInlineEndWidth`, `borderBlockStartWidth`, `borderBlockEndWidth` | Nonnegative lengths; thin=1px, medium=3px, thick=5px. No percentages. Physical all-edge shorthand takes 1–4 values; logical pairs take 1–2. | [styles/radii][css/border/styles-radius], border unit tests, [logical sides][css/layout/logical-sides-matrix]; `flow_relative_sides_are_the_physical_ones_of_every_writing_mode_and_direction`, `physical_border_longhands_set_their_own_side` |
| `borderStyle`, `borderTopStyle`, `borderRightStyle`, `borderBottomStyle`, `borderLeftStyle`, `borderInlineStyle`, `borderBlockStyle`, `borderInlineStartStyle`, `borderInlineEndStyle`, `borderBlockStartStyle`, `borderBlockEndStyle` | none, hidden, solid, double, dotted, dashed, groove, ridge, inset, outset. None/hidden draw no border.                                      | [styles/radii][css/border/styles-radius] targets solid/double/dotted/dashed; other styles have parser evidence, targeted browser cases not mapped, `flow_relative_border_styles_and_colors_reach_their_physical_sides`, `physical_border_longhands_set_their_own_side` |
| `borderColor`, `borderTopColor`, `borderRightColor`, `borderBottomColor`, `borderLeftColor`, `borderInlineColor`, `borderBlockColor`, `borderInlineStartColor`, `borderInlineEndColor`, `borderBlockStartColor`, `borderBlockEndColor` | Colors/currentColor, physical and logical shorthand spreading.                                                                              | [current mixes][css/color/current-mix], [bidi][css/text/bidi], border unit tests, `flow_relative_border_styles_and_colors_reach_their_physical_sides`, `physical_border_longhands_set_their_own_side` |
| `borderRadius`, `borderTopLeftRadius`, `borderTopRightRadius`, `borderBottomRightRadius`, `borderBottomLeftRadius`                                                                                                                     | Nonnegative lengths/percentages; elliptical radii, 1–4 corners and slash-separated axes.                                                    | [styles/radii][css/border/styles-radius], [rounded clipping][css/shadow/rounded-clip]                                                             |
| `borderStartStartRadius`, `borderStartEndRadius`, `borderEndEndRadius`, `borderEndStartRadius`                                                                                                                                         | Logical corner radii resolved with writing mode/direction.                                                                                  | [bidi][css/text/bidi]; exhaustive corner/direction combinations not mapped, `flow_relative_sizes_overflow_and_corners_follow_the_writing_mode` |
| `outline`, `outlineWidth`, `outlineStyle`, `outlineColor`                                                                                                                                                                              | Border-like width/style/color. Outline style excludes hidden and auto focus rings.                                                          | Outline unit tests; [Tailwind borders](../tests/cases/tailwind/borders/case.json)                                                                 |
| `outlineOffset`                                                                                                                                                                                                                        | Signed length, no percentage.                                                                                                               | Outline unit tests; [Tailwind borders](../tests/cases/tailwind/borders/case.json)                                                                 |
| `boxShadow`                                                                                                                                                                                                                            | none or comma-separated shadows with x/y offsets, optional nonnegative blur, signed spread, color/currentColor and inset.                   | [outer/inset][css/shadow/outer-inset], [blur][css/shadow/blur], [rounded clip][css/shadow/rounded-clip]                                           |

Coverage: [unequal corners][css/border/unequal-corners],
[mixed joins][css/border/mixed-joins], [logical corners][css/border/logical-corners]
and [outline layers][css/shadow/outline-layers] cover mixed widths/styles,
logical mapping, clipping, transforms and inner/outer shadows. Dynamic controls
change the same nodes. Dash phase and edge antialiasing allowances are documented
per case and paired with strict Valle images; arbitrary widths are not exhausted.

## Compositing, filters and masks

Implementation: [filter](../crates/valle-engine/src/style/filter.rs),
[shapes](../crates/valle-engine/src/style/shape.rs),
[layers](../crates/valle-engine/src/paint.rs).
The supported blend set is normal, multiply, screen, overlay, darken, lighten,
color-dodge, color-burn, hard-light, soft-light, difference, exclusion, hue,
saturation, color, luminosity and plus-lighter. Plus-darker is not in the table.

| Properties                               | Implemented scope and restrictions                                                                                                                                                                                                                                              | Existing evidence                                                                                                                                                                                                               |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `opacity`                                | Number/percentage clamped to 0–1; applied to the composited group.                                                                                                                                                                                                              | [group opacity][css/compositing/group-opacity], [stacking][css/compositing/stacking-opacity]                                                                                                                                    |
| `mixBlendMode`, `isolation`              | Blend set above; isolation auto/isolate.                                                                                                                                                                                                                                        | [isolation/blend][css/compositing/isolation-blend], [rounded blend][css/compositing/rounded-blend], [Tailwind blending](../tests/cases/tailwind/blending/case.json)                                                             |
| `filter`                                 | none or ordered blur, brightness, contrast, grayscale, hue-rotate, invert, saturate, sepia, drop-shadow, opacity; also the six Valle functions described below. A single url(#id) graph is limited to supported SVG descendants, not CSS boxes/outer SVG or URL/function lists. | [function order][css/filter/order], [opacity][css/filter/opacity], [blur/overflow][css/filter/blur-overflow], [SVG CSS effects](../tests/cases/svg/css-effects/inline/case.json)                                                |
| `backdropFilter`                         | Same function list applied to the backdrop; no SVG filter URL.                                                                                                                                                                                                                  | [backdrop/blend basic][css/compositing/backdrop-blend/basic], [controls][css/compositing/backdrop-blend/controls], [nested][css/compositing/backdrop-blend/nested], [empty bounds][css/compositing/backdrop-blend/empty-bounds] |
| `clipPath`                               | none; inset/circle/ellipse/polygon/path/shape; border/padding/content reference boxes. Local SVG clip references apply in SVG. No arbitrary HTML reference to a clip definition.                                                                                                | [responsive shape][css/mask/shape], [transform/clip][css/transform/clip], [SVG clip intersection](../tests/cases/svg/clip-intersection/inline/case.json)                                                                        |
| `maskImage`                              | Layers of none, gradients or declared resource images.                                                                                                                                                                                                                          | [clip/mask][css/mask/clip], [mask composites][css/mask/composite], mask unit tests                                                                                                                                              |
| `maskSize`, `maskPosition`, `maskRepeat` | Per-layer image sizing, positioning and repeat forms, as supported by their background counterparts.                                                                                                                                                                            | Mask unit tests; [Tailwind masks](../tests/cases/tailwind/masks/case.json); rendered per-longhand edge cases not mapped                                                                                                         |
| `maskMode`                               | alpha, luminance, match-source; match-source maps to alpha for this image-layer path.                                                                                                                                                                                           | `masks_are_layers_of_images_shown_by_alpha_or_luminance`; [Tailwind masks](../tests/cases/tailwind/masks/case.json)                                                                                                             |
| `maskComposite`                          | add, subtract, intersect, exclude, as layer lists.                                                                                                                                                                                                                              | [four operators][css/mask/composite]                                                                                                                                                                                            |
| `maskClip`, `maskOrigin`                 | Border/padding/content box. No no-clip/fill-box/stroke-box/view-box values in the CSS image-layer path.                                                                                                                                                                         | Mask unit tests; [Tailwind masks](../tests/cases/tailwind/masks/case.json)                                                                                                                                                      |

Standard filter parameters:

- blur takes a nonnegative length (default 0), without percentages.
- brightness/contrast/saturate take a nonnegative number or percentage, without
  an upper clamp. Grayscale/invert/opacity/sepia clamp that amount to 1. These
  functions default to 1 when their argument is omitted.
- hue-rotate takes an angle (default 0).
- drop-shadow takes x/y offsets, optional nonnegative blur and a color/currentColor;
  no spread or inset. It uses the supported CSS length parser.

Functions retain authored order. Argument and error tests live in `filter.rs`
and `declaration.rs`.

Valle extensions are **not additional CSS properties**: bloom, displacement,
film-grain, chromatic-aberration, lens-distortion and radial-blur are values of
filter/backdropFilter. [Motion](motion.md#valle-filters) documents their parameters.
`motionBlur` is a separate Valle-only property; `xmlSpace` belongs to SVG/XML.
Neither is included in the 284-name inventory.

Coverage: [blend families][css/compositing/blend-families] checks all 16 blend
modes against absent/inverted backdrops; [backdrop roots][css/compositing/backdrop-roots]
checks nested opacity/filter/mask/isolation controls; and
[three-layer masks][css/mask/three-layer-composites] crosses all four operators
with alpha/luminance masks. Existing blur and transformed-bound controls remain
in place. Matching Native/Web pixels do not replace these independent oracles.

## Transforms and motion paths

Implementation: [transform](../crates/valle-engine/src/style/transform.rs),
[offset](../crates/valle-engine/src/style/offset.rs), and paint transforms.
Unit tests in `declaration.rs` cover 2D/3D functions, origin z, individual transform
order and path longhands. These are not all browser-rendered combinations.

| Properties                             | Implemented scope and restrictions                                                                                                                | Existing evidence                                                                                                                                                                                 |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `transform`                            | none or ordered matrix/matrix3d, translate/translateX/Y/Z/3d, scale/scaleX/Y/Z/3d, rotate/rotateX/Y/Z/3d, skew/skewX/Y and perspective functions. | [origin/order][css/transform/origin-order], [clip][css/transform/clip], [Tailwind transforms](../tests/cases/tailwind/transforms/case.json)                                                       |
| `translate`, `rotate`, `scale`         | Individual transforms including supported 3D forms; applied before the transform list.                                                            | `translate_rotate_and_scale_come_before_transform`; [SVG CSS transforms](../tests/cases/svg/css-transforms/inline/case.json), [Tailwind transforms](../tests/cases/tailwind/transforms/case.json) |
| `transformOrigin`                      | Position with optional z length; percentage x/y use the transform reference box.                                                                  | [origin/order][css/transform/origin-order], transform unit tests                                                                                                                                  |
| `transformBox`                         | content-box, border-box, fill-box, stroke-box, view-box; SVG mappings are context-dependent.                                                      | [SVG CSS transforms](../tests/cases/svg/css-transforms/inline/case.json) and file counterpart                                                                                                     |
| `perspective`, `perspectiveOrigin`     | none or supported nonnegative distance, and a 2D origin; not a browser camera API.                                                                | 3D transform unit tests; [Tailwind transforms](../tests/cases/tailwind/transforms/case.json)                                                                                                      |
| `backfaceVisibility`, `transformStyle` | visible/hidden; flat/preserve-3d, subject to grouping effects that flatten a scene.                                                               | [scene isolation][css/compositing/scene-blend-isolation], [backdrop planes][css/compositing/backdrop-blend/planes], [Tailwind transforms](../tests/cases/tailwind/transforms/case.json)           |
| `offsetPath`                           | none, path, ray, inset, circle, ellipse, polygon, shape, and supported border/padding/content reference boxes. No url() path reference.           | [shape path][css/transforms/shape-path]; `motion_paths_take_paths_rays_and_shapes_with_their_longhands`                                                                                           |
| `offsetDistance`, `offsetRotate`       | Signed length/percentage along the route; auto/reverse with optional angle, or a fixed angle.                                                     | [shape path][css/transforms/shape-path], motion-path unit tests                                                                                                                                   |
| `offsetAnchor`, `offsetPosition`       | Anchor auto/position; route start normal/auto/position.                                                                                           | Motion-path unit tests; independent rendered anchor/start matrix not mapped                                                                                                                       |

Coverage: [order and clipping][css/transform/order-clipping],
[perspective groups][css/transform/perspective-groups] and
[path extents][css/transforms/path-extents] cross origins, negative scales,
transform order, flattening, anchors and out-of-range distances. Each has a
changing-state companion. These do not exhaust every motion-path shape or
perspective depth.

## Fonts

Implementation and direct unit tests:
[font](../crates/valle-engine/src/style/font.rs),
[text shaping](../crates/valle-engine/src/text.rs).
Family/axis/feature support depends on the pinned or explicitly loaded font;
accepting a feature tag does not create glyphs missing from that font.

| Properties                                     | Implemented scope and restrictions                                                                                                                                                            | Existing evidence                                                                                                                                                                 |
| ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `font`                                         | Size and family with supported style, small-caps, weight, stretch and optional line-height; omitted longhands reset. No system-font keywords.                                                 | [shorthand][css/text/font-shorthand]; `font_shorthand_sets_and_resets_its_longhands`                                                                                              |
| `fontFamily`                                   | Ordered declared font-family names and built-in sans-serif, serif, monospace, emoji. Other browser/system generic families are rejected. No discovery of arbitrary installed fonts.           | [mixed runs][css/text/mixed-runs], font-family parsing/shaping tests; [mixed-script breaking][css/text/breaking-products]                                                                  |
| `fontSize`                                     | Nonnegative length/percentage; xx-small=9px, x-small=10px, small=13px, medium=16px, large=18px, x-large=24px, xx-large=32px; larger/smaller multiply/divide parent size by 1.2. No xxx-large. | [keywords][css/text/font-size-keywords]; `size_keywords_resolve_before_relative_units_and_shorthand`                                                                              |
| `fontSizeAdjust`                               | none or nonnegative x-height/em ratio; adjusts glyphs/ex, not computed fontSize/em.                                                                                                           | [size adjust][css/text/font-size-adjust]; `size_adjust_inherits_and_font_shorthand_resets_it`                                                                                     |
| `fontWeight`                                   | 1–1000, normal/bold/bolder/lighter; font matching and synthesis follow available faces.                                                                                                       | [mixed runs][css/text/mixed-runs], [synthesis][css/text/font-synthesis], [Tailwind text](../tests/cases/tailwind/text/case.json); [relative inheritance][css/text/relative-inheritance] |
| `fontStretch`                                  | Nine width keywords from ultra-condensed to ultra-expanded, or nonnegative percentage.                                                                                                        | `font_stretch_is_a_keyword_or_a_percentage`; [Tailwind text](../tests/cases/tailwind/text/case.json)                                                                              |
| `fontStyle`                                    | normal, italic, oblique with optional angle in −90deg…90deg; default oblique is 14deg.                                                                                                        | [mixed runs][css/text/mixed-runs], [synthesis][css/text/font-synthesis]; `font_style_is_italic_or_oblique_by_an_angle`                                                            |
| `fontSynthesis`                                | none, weight, style, or weight style. Controls simulated bold/italic; does not disable native faces or variable axes.                                                                         | [synthesis][css/text/font-synthesis]; `synthesis_accepts_only_css_fonts_three_permissions`                                                                                        |
| `fontKerning`                                  | normal, auto, none; normal resets the longhand's feature override.                                                                                                                            | `font_variants_set_their_features_one_longhand_at_a_time`; [paired features][css/text/font-features]                                                                             |
| `fontFeatureSettings`, `fontVariationSettings` | normal or quoted 4-character OpenType tags; nonnegative integer/on/off feature values, numeric variation-axis values.                                                                         | `font_features_are_quoted_tags_with_their_values`, `font_variations_are_quoted_axes_with_numbers`; [ex/weight axis][css/values/font-ex]                                           |
| `fontVariant`                                  | normal/none or supported longhand keyword combinations; not arbitrary feature-value functions.                                                                                                | Font-variant unit tests; [caps][css/text/caps], [font position][css/text/font-position]                                                                                           |
| `fontVariantLigatures`                         | normal/none; common, discretionary, historical and contextual enable/disable keywords.                                                                                                        | Font-variant unit tests; [paired feature controls][css/text/font-features]                                                                                                                 |
| `fontVariantCaps`                              | normal, small-caps, all-small-caps, petite-caps, all-petite-caps, unicase, titling-caps. Native forms, supported fallback forms, then synthesized small capitals where needed.                | [caps][css/text/caps]; synthetic/caseless shaping unit tests                                                                                                                      |
| `fontVariantNumeric`                           | normal; lining/oldstyle, proportional/tabular, diagonal/stacked fractions, ordinal, slashed-zero.                                                                                             | Font-variant unit tests; [Tailwind text](../tests/cases/tailwind/text/case.json); [paired browser controls][css/text/font-features]                                                         |
| `fontVariantEastAsian`                         | normal; jis78/83/90/04, simplified/traditional, full-width/proportional-width, ruby.                                                                                                          | Font-variant unit tests; [keyword controls][css/text/font-features] (actual substitutions depend on the pinned font)                                                                                                        |
| `fontVariantAlternates`                        | normal, historical-forms. Feature-value functions such as stylistic()/styleset() are not supported.                                                                                           | Font-variant unit tests; targeted rendering not mapped                                                                                                                            |
| `fontVariantPosition`                          | normal, sub, super; synthesize a run when the font lacks the complete native variant. Separate from verticalAlign, which moves inline boxes.                                                  | [font position][css/text/font-position], font/text unit tests                                                                                                                     |

Coverage: [feature controls][css/text/font-features] pairs 44 accepted keyword
settings with default-font controls; [axes and synthesis][css/text/axes-synthesis]
and [relative inheritance][css/text/relative-inheritance] cover variable axes,
size adjustment and nested runs. Missing substitutions are explicitly distinct
from unsupported CSS. The pinned fonts do not prove every OpenType feature on
every font; whole-run missing-feature fallback also has direct shaping tests.

## Text layout and decoration

Implementation: [font](../crates/valle-engine/src/style/font.rs),
[inline alignment](../crates/valle-engine/src/style/inline.rs),
[emphasis](../crates/valle-engine/src/style/emphasis.rs),
[text](../crates/valle-engine/src/text.rs), and inline layout/painting.

| Properties                                                                       | Implemented scope and restrictions                                                                                                                                            | Existing evidence                                                                                                                                                       |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `color`                                                                          | Color/currentColor; inherited by text and available to other paints.                                                                                                          | [current mixes][css/color/current-mix], [sRGB][css/color/srgb-alpha]                                                                                                    |
| `lineHeight`                                                                     | normal, nonnegative unitless factor, length or percentage. A unitless factor inherits differently from a resolved length.                                                     | [mixed runs][css/text/mixed-runs], [vertical align][css/text/vertical-align], font inheritance unit tests                                                               |
| `textAlign`, `textAlignLast`                                                     | start, end, left, right, center, justify; last also auto.                                                                                                                     | [alignment][css/text/alignment], [paragraph][css/text/paragraph]                                                                                                        |
| `verticalAlign`                                                                  | baseline, middle, sub, super, text-top, text-bottom, top, bottom, signed length/percentage of own line-height. Not inherited unless explicitly requested.                     | [12 alignment controls][css/text/vertical-align], [inline writing][css/text/inline-writing]; `inline_alignment_parses_units_and_inherits_only_explicitly`               |
| `textIndent`                                                                     | Signed length/percentage; first-line indent. No hanging/each-line keywords.                                                                                                   | [spacing/indent][css/text/spacing-indent], [Tailwind paragraphs](../tests/cases/tailwind/paragraphs/case.json)                                                          |
| `letterSpacing`, `wordSpacing`                                                   | normal or signed length; percentages are not accepted.                                                                                                                        | [spacing/indent][css/text/spacing-indent], [ex units][css/values/font-ex]                                                                                               |
| `whiteSpace`, `whiteSpaceCollapse`                                               | normal/nowrap/pre/pre-wrap/pre-line and supported collapse+wrap combinations. Collapse values: collapse, preserve, preserve-breaks, break-spaces. No discard/preserve-spaces. | [preserved spaces][css/text/preserved-spaces], [word wrap][css/text/word-wrap]; `white_space_sets_its_collapsing_and_wrapping`                                          |
| `textWrap`, `textWrapMode`, `textWrapStyle`                                      | wrap/nowrap; auto, balance, stable, pretty; shorthand accepts mode and style. Stable uses the stateless auto behavior.                                                        | [paragraph][css/text/paragraph], [Tailwind text](../tests/cases/tailwind/text/case.json); `text_wrap_sets_its_mode_and_style_or_refuses`                                |
| `wordBreak`                                                                      | normal, break-all, keep-all, break-word; no auto-phrase/manual.                                                                                                               | [break-all][css/text/wrapping], [paragraph][css/text/paragraph]                                                                                                         |
| `overflowWrap`, `wordWrap`                                                       | normal, anywhere, break-word; wordWrap aliases overflowWrap.                                                                                                                  | [overflow wrap][css/text/overflow-wrap]; alias-specific rendering not separately counted                                                                                |
| `lineBreak`                                                                      | auto, normal, strict, loose, anywhere; auto uses strict in this implementation.                                                                                               | [paragraph][css/text/paragraph]                                                                                                                                         |
| `writingMode`, `textOrientation`                                                 | horizontal-tb, vertical-rl, vertical-lr; mixed/upright/sideways glyph orientation. No sideways-rl/sideways-lr writing modes.                                                  | [vertical text][css/text/vertical], [vertical container][css/text/vertical-container], vertical layout cases                                                            |
| `direction`, `unicodeBidi`                                                       | ltr/rtl; normal, embed, bidi-override, isolate, isolate-override, plaintext.                                                                                                  | [bidi][css/text/bidi], [SVG text direction](../tests/cases/svg/text-direction/inline/case.json)                                                                         |
| `textCombineUpright`                                                             | none/all; no digits keyword with a count.                                                                                                                                     | [combined vertical text][css/text/combine]                                                                                                                              |
| `textTransform`                                                                  | none, uppercase, lowercase, capitalize. No full-width/full-size-kana forms.                                                                                                   | [Tailwind text](../tests/cases/tailwind/text/case.json); independent Unicode casing image matrix not mapped                                                             |
| `textOverflow`                                                                   | clip/ellipsis; no custom string or two-value form. Requires the applicable overflow/no-wrap configuration.                                                                    | [ellipsis][css/text/ellipsis]; `text_overflow_is_clip_or_an_ellipsis_of_the_box_alone`                                                                                  |
| `WebkitBoxOrient`, `WebkitLineClamp`                                             | horizontal/vertical/inline-axis/block-axis; none or positive line count. Line clamp requires a vertical -webkit-box Text. Not a general legacy box layout implementation.     | `line_clamp_needs_a_vertical_webkit_box_of_a_text`; [Tailwind text](../tests/cases/tailwind/text/case.json), [paragraphs](../tests/cases/tailwind/paragraphs/case.json) |
| `textEmphasis`, `textEmphasisStyle`, `textEmphasisColor`, `textEmphasisPosition` | none, filled/open dot/circle/double-circle/triangle/sesame or a quoted custom mark; color; over/under with optional left/right placement.                                     | [emphasis][css/text/emphasis], [combined text][css/text/combine]                                                                                                        |
| `textDecoration`, `textDecorationLine`                                           | none or underline/overline/line-through combinations; shorthand also sets style/color/thickness.                                                                              | [CJK decorations][css/text/decoration-cjk], [Tailwind text](../tests/cases/tailwind/text/case.json); decoration unit tests                                              |
| `textDecorationColor`, `textDecorationStyle`                                     | Color/currentColor; solid, double, dotted, dashed, wavy.                                                                                                                      | [CJK decorations][css/text/decoration-cjk], [SVG text baselines](../tests/cases/svg/text-baselines/inline/case.json)                                                    |
| `textDecorationThickness`                                                        | auto or nonnegative length/percentage; from-font is explicitly rejected.                                                                                                      | `text_decoration_sets_its_longhands_or_refuses`; [CJK decorations][css/text/decoration-cjk]                                                                             |
| `textDecorationSkipInk`                                                          | auto, all, none; auto treats CJK differently from Latin descenders.                                                                                                           | [CJK decorations][css/text/decoration-cjk]; `ink_skipping_distinguishes_auto_all_and_none_and_inherits`                                                                 |
| `textUnderlineOffset`, `textUnderlinePosition`                                   | Offset auto or signed length/percentage; position auto or under with optional left/right physical side.                                                                       | [underline position][css/text/underline-position]; `underline_position_parses_and_is_not_reset_by_decoration`                                                           |
| `WebkitTextFillColor`, `WebkitTextStrokeColor`                                   | Color/currentColor for text fill/stroke.                                                                                                                                      | [paint order][css/text/paint-order], [CJK decorations][css/text/decoration-cjk]                                                                                         |
| `WebkitTextStroke`, `WebkitTextStrokeWidth`                                      | Nonnegative length or thin/medium/thick, plus color in the shorthand. Centered glyph-outline stroke.                                                                          | [paint order][css/text/paint-order]; `text_stroke_sets_its_longhands_or_refuses`                                                                                        |
| `textShadow`                                                                     | none or multiple x/y/optional-blur/color shadows; no spread or inset.                                                                                                         | `text_shadows_inherit_in_px_with_currentcolor_kept`; [Tailwind shadows](../tests/cases/tailwind/shadows/case.json)                                                      |

Span supports sliced inline backgrounds/borders/padding across wrapped lines,
including nested/empty spans; it is not a separate width/height/margin box.
`boxDecorationBreak` is not a supported property. Inline View/Image/SVG boxes are
separate atomic items and use the paragraph's placement rules.

Coverage: [breaking products][css/text/breaking-products] checks 108 selected
white-space/word-break/overflow-wrap combinations; [bidi atoms][css/text/bidi-inline-atoms]
crosses directions, writing modes and inline alignment; and
[nested decorations][css/text/nested-decorations] checks inherited and local
lines through nested runs. Dynamic fixtures revisit prior states to detect stale
shaping/layout caches. Arbitrary scripts and all Unicode break classes are not exhausted.

## SVG presentation styles

These names are included in the 284-name count because they use the same style
table. Their SVG-only semantics do not become box CSS capabilities. Geometry
attributes such as d, cx and viewBox, and SVG tag counts, are not part of this
inventory. See [Motion's SVG sections](motion.md) for element syntax.

Implementation: [SVG](../crates/valle-engine/src/svg.rs),
[SVG text](../crates/valle-engine/src/svg/text.rs),
[declarations](../crates/valle-engine/src/style/declaration.rs).
The linked inline cases have adjacent file cases unless noted otherwise.

| Properties                                                           | Implemented scope and restrictions                                                                                                               | Existing evidence                                                                                                                                                                                        |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fill`, `stroke`                                                     | none, colors/currentColor, local paint-server references. No context-fill/context-stroke or paint fallback suffix.                               | [inheritance](../tests/cases/svg/use-inheritance/inline/case.json), [current mixes][css/color/current-mix]                                                                                               |
| `fillOpacity`, `strokeOpacity`, `stopOpacity`, `floodOpacity`        | Number/percentage clamped to 0–1; fill/stroke alpha is separate from group opacity.                                                              | [paint opacity](../tests/cases/svg/group-paint-opacity/inline/case.json), filter/gradient cases                                                                                                          |
| `fillRule`, `clipRule`                                               | nonzero/evenodd. Clip rule is independent of fill rule.                                                                                          | [clip rules](../tests/cases/svg/clip-rule-transform/inline/case.json), [viewBox/dashes](../tests/cases/svg/viewbox-dashes/inline/case.json)                                                              |
| `paintOrder`                                                         | normal or a nonrepeating subset/order of fill, stroke, markers; omitted operations follow in normal order. Also used by Text paint.              | [Text paint order][css/text/paint-order]; `inline_and_file_shapes_order_fill_stroke_and_markers` in [SVG painting](../crates/valle-engine/src/paint/svg.rs); targeted SVG browser case not mapped        |
| `strokeWidth`, `strokeLinecap`, `strokeLinejoin`, `strokeMiterlimit` | Nonnegative SVG length/percentage; butt/round/square caps; miter/round/bevel joins; miterlimit at least 1. No arcs/miter-clip joins.             | [caps/joins](../tests/cases/svg/stroke-caps-joins/inline/case.json)                                                                                                                                      |
| `strokeDasharray`, `strokeDashoffset`                                | none or nonnegative dash-length lists; signed offset. SVG percentages and pathLength calibration use the SVG geometry path.                      | [dash offsets](../tests/cases/svg/dashes-offset/inline/case.json), [viewBox/dashes](../tests/cases/svg/viewbox-dashes/inline/case.json)                                                                  |
| `vectorEffect`                                                       | none/non-scaling-stroke; applies to stroke scaling inside SVG, not an exemption from outer CSS image transforms.                                 | [non-scaling stroke](../tests/cases/svg/non-scaling-stroke/inline/case.json)                                                                                                                             |
| `marker`, `markerStart`, `markerMid`, `markerEnd`                    | none or local marker reference; shorthand assigns all three positions.                                                                           | [markers](../tests/cases/svg/markers/inline/case.json)                                                                                                                                                   |
| `stopColor`, `floodColor`, `lightingColor`                           | Colors/currentColor on gradient stops and filter primitives.                                                                                     | [interpolation](../tests/cases/svg/color-interpolation/inline/case.json), [diffuse](../tests/cases/svg/filter-diffuse/inline/case.json), [specular](../tests/cases/svg/filter-specular/inline/case.json) |
| `colorInterpolation`, `colorInterpolationFilters`                    | auto, sRGB, linearRGB; auto selects sRGB for gradients and linearRGB for filters.                                                                | [gradient interpolation](../tests/cases/svg/color-interpolation/inline/case.json), SVG filter cases                                                                                                      |
| `mask`, `maskType`                                                   | mask is none/url(#id) for an SVG mask, **not the CSS image-mask shorthand**. maskType is luminance/alpha. Boxes use maskImage and its longhands. | [clip/mask/opacity](../tests/cases/svg/clip-mask-opacity/inline/case.json), [mask modes](../tests/cases/svg/masks/inline/case.json)                                                                      |
| `textAnchor`                                                         | start/middle/end, accounting for direction and SVG text chunks.                                                                                  | [SVG text](../tests/cases/svg/text/inline/case.json), [direction](../tests/cases/svg/text-direction/inline/case.json)                                                                                    |
| `dominantBaseline`, `alignmentBaseline`, `baselineShift`             | SVG baseline keywords and baseline/sub/super or signed length/percentage shifts; details below. These do not replace CSS verticalAlign.          | [SVG baselines](../tests/cases/svg/text-baselines/inline/case.json), [vertical text](../tests/cases/svg/vertical-text/inline/case.json)                                                                  |

Baseline keywords: auto, alphabetic, middle, central, hanging, mathematical,
text-before-edge/text-top, text-after-edge/text-bottom and ideographic.
AlignmentBaseline additionally accepts baseline; auto/baseline clear its override.
BaselineShift takes baseline, sub, super or a signed SVG length/percentage.

Coverage: [SVG combinations](../tests/cases/svg/combinations/) retains matching
inline/file inputs for marker strokes and paint order, gradient/clip/filter
chains, text paths and baseline shifts, and viewports/use references. All four
families have independent browser captures; two changing fixtures check marker
styles and file-image placement. Input-path, Native/Web and specification checks
remain distinct evidence.

## Deliberately excluded and currently limited behavior

The property table's diagnostics are the implementation authority. Exclusion is
not always about determinism: some items are product scope or missing layout
capabilities.

- No browser event/interaction/scroll model: hover/focus states, cursor, pointer
  events, selection, caret, touch, scroll containers and sticky positioning.
- No CSS animation or transition timeline. Animate frame values in TSX.
- No author CSS variables, CSS-wide revert/revert-layer or all shorthand.
- No floats, multicolumn/table/list layout, counters, generated content or paged
  fragmentation. Block, Flex, Grid and explicit Text paragraphs are supported.
- No border-image, zoom, subgrid/masonry, image-set/cross-fade, container queries
  or arbitrary HTML stylesheet layout. SVG file stylesheets have their own
  supported selector/cascade subset.
- Browser hints such as will-change, contain and content-visibility are rejected.
- Partial properties remain partial: min/max intrinsic keywords, mask shorthand,
  visibility collapse, font variants and text decoration limits are called out
  above. This inventory records implemented syntax; it does not classify every
  feature's standards maturity.

## Combination coverage and gaps

The four retained batches add **298 static factor tuples** in 14 browser fixtures
and **24 sampled frames** in three dynamic fixtures. A tuple is one authored
combination, not one supported property or a global CSS coverage percentage.

| Batch                   | Retained checks                                                                                                                                                                                                                                                                           | Independent oracle                                                                                                                |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Layout: 80 tuples       | [constraints][css/flex/constraints-matrix] (24), [wrap boundaries][css/flex/wrap-thresholds] (36), [nested sizing][css/layout/nested-sizing] (8), [logical containing blocks][css/position/logical-containing-block] (12)                                                                 | Arithmetic expectations for 236 boxes; all three browser images and measurements agree                                            |
| Text: 90 tuples         | [wrapping][css/text/wrap-matrix] (24), [atomic alignment][css/text/alignment-matrix] (12), [decorations][css/text/decoration-matrix] (30), [truncation][css/text/truncation-matrix] (12), [mixed sizes][css/text/mixed-size-matrix] (12)                                                  | Pinned font advances, explicit lines/box heights, baseline unit assertions and reviewed browser pictures                          |
| Compositing: 128 tuples | [filter/blend][css/compositing/filter-blend-matrix] (36), [clip/mask][css/compositing/clip-mask-matrix] (36), [nested groups][css/compositing/nested-matrix] (32), [transformed filter bounds][css/filter/transformed-bounds] (18), [visibility][css/compositing/visibility-backdrop] (6) | Reviewed pictures and difference maps, solid-color/empty controls; mutations must reject missing filters/masks/clips              |
| Dynamic: 24 frames      | [layout][css/flex/dynamic-constraints], [text inheritance][css/text/dynamic-inheritance], [layers][css/compositing/dynamic-layers], eight frames each                                                                                                                                     | Independent wrap/size/line-height assertions; reviewed PNGs; fresh, forward, reverse and alternating seeks; Native/WebGPU entries |

Each static matrix enumerates the full product of its declared factors. The
factor values and dimensions are explicit in its TSX and README. The matrices
cover selected legal values, not every value or cross-product of CSS properties.

The inline geometry unit test additionally crosses **3 writing modes × 2
directions × 3 display modes × 4 alignments = 72 combinations**, with four
visits each. Display and alignment are independent; these are not 72 separate
browser reference pictures.

New regressions exposed and fixed RTL ellipsis, mixed-size vertical baselines
and line extents, opacity-group flattening across backdrop reads, and hidden
boxes painting backdrop filters. The visibility/visible-child interaction is
not settled: [FXTF issue 455](https://github.com/w3c/fxtf-drafts/issues/455)
describes the same browser disagreement. That case selects Firefox's proposed
background-like behavior and retains the Chrome/WebKit alternatives.

Glyph antialiasing, blur falloff and fractional clipping coverage are reviewed
separately from semantic errors. The decoration matrix also has a strict Valle
baseline because browser underline placement and dash/wave phases differ.
Mutation tests verify that removing effects still fails the combined checks.

All fixtures, numerical expectations, browser HTML/PNG/JSON and accepted dynamic
PNGs are permanent. Actual/difference pictures stay under `target/test-results`.
Browser cases use `at: [0]`; dynamic cases do not pretend to be live browser
oracles. Their browser controls and numerical checks establish semantics, while
fresh/seek and Native/Web checks establish consistency.

The follow-up coverage adds **482 CSS combinations**, **58 SVG combinations
through two input paths**, and **152 dynamic samples**. It retains **74 new
fixtures** in total, including the 20 generated-tree fixtures.

| Follow-up batch | Static combinations | Dynamic samples | Evidence |
| --- | ---: | ---: | --- |
| Units and sizing | 60 | 16 | Independent box arithmetic, three browsers, intrinsic/percentage regression tests |
| Borders and shadows | 66 | 16 | Three browsers, reviewed strict Valle images, missing-effect mutations |
| Backgrounds, images and colors | 74 | 32 | Three browsers, tile arithmetic and independent positioning expectations |
| Fonts and text | 190 | 32 | Pinned fonts, paired feature controls, shaping/geometry regressions and reviewed images |
| Nested compositing | 52 | 16 | All blend modes, backdrop-root controls and alpha/luminance mask operators |
| Transforms and paths | 40 | 24 | Three browsers, origin/order/clipping controls and changing states |
| SVG cross-module inputs | 58 × 2 paths | 16 | Inline/file equality, three browsers and marker-unit geometry regression |
| Generated layout trees | 1,000 trees | — | 9,742 boxes; 116,904 scalar assertions; exact three-browser geometry and images |

The new regressions fixed indefinite percentage heights, intrinsic width
constraints, rounded-background positioning, bidi order around inline boxes and
vertical line breaking, and non-scaling marker sizes. Seek comparisons now
resolve font handles to font content and variation settings: allocation order
is not an observable rendering change, while glyph data and pixels still compare.
The full regression suite also exposed control-only bidi runs being counted as
extra atoms in box-only vertical paragraphs. Those runs are now excluded, with
three writing modes and both directions checked explicitly.

The existing [SVG background case][css/backgrounds/svg-image] now selects the
retained Firefox capture for round positioning after a specification review.
Its README records the resized-tile arithmetic and the Chrome/WebKit offset;
a strict Valle image and a missing-round mutation protect against geometric
regressions. Its original browser captures have not been rewritten.

Capture metadata for these new static references is retained once in
[coverage-environment.json](../tests/baselines/browser/coverage-environment.json),
with an explicit fixture list. Older references retain their original metadata;
they are not represented as recaptured with the newer browsers.

### Runtime validation

All 74 new fixtures are registered in the existing Native/Chromium WebGPU
comparison. Dynamic ones revisit frames in the order `0,7,1,6,2,5,3,4,0`;
the native runner also checks fresh, forward, reverse and alternating visits.
On macOS with Apple M2 Pro / Metal, `cargo xtask test` passes: **776 Rust test
functions**, **333 scene cases**, **50 Bun tests**, **201 Native/WebGPU fixtures
with 538 frame visits**, and the browser playback/export and Studio checks.
The 74 new fixtures account for 226 of those visits; their largest native/web
channel difference is 1. All 20 generated-tree images match exactly across the
two runtimes. `cargo xtask lint` and `cargo xtask fmt --check` also pass.

These counts describe different layers, not quantities to add together. A
passed Rust case-runner function contains many scenes and numerical assertions.

The existing CI already runs native cases on Linux and Windows with the pinned
Lavapipe, and native plus browser tests on macOS. This local, unpushed change
has not been executed on Linux or Windows. No new platform pass is claimed.
Fixed-seed trees explore a declared grammar, not all possible CSS combinations;
minimized failures must become permanent regressions rather than automatically
accepted screenshots.

## Keeping this inventory current

When changing a supported property, update its accepted values/restrictions and
link the precise regression. If a test only parses, say so; if a rendering test
only covers one value, do not mark the entire property complete. Add combinations
under the feature they exercise, following the existing cases directory layout.

The CSS case index below is a snapshot of the checked-in `case.json` definitions.
Its box/text counts describe explicit expectations; its browser choice records
the reviewed baseline, not a fresh browser capture. Native/Web means that
`player.ts` lists the scene. Actual execution results are recorded under Runtime
validation above; registration alone is not a passing result.

<!-- CSS case index follows. Keep this in sync when cases are added or removed. -->

## CSS scene index

Counts describe explicit expectations per sampled frame. A Native/Web entry means
the fixture is registered in `web/tests/player.ts`; it is not evidence that every
browser implements WebGPU. Static browser cases sample frame 0; dynamic rows
retain the listed number of sampled frames and run seek checks automatically.

| Case | Frames | Boxes / text objects | Native/Web entry | Assertion |
| --- | ---: | ---: | --- | --- |
| [background/dynamic-layer-lists][css/background/dynamic-layer-lists] | 8 | 0 / 0 | Listed | State changes and restoration for css/background/layer-lists. The eight states reuse the independently reviewed static combinations. |
| [background/dynamic-rounded-position][css/background/dynamic-rounded-position] | 8 | 0 / 0 | Listed | State changes and restoration for css/background/rounded-position. The eight states reuse the independently reviewed static combinations. |
| [background/fractional-repeat][css/background/fractional-repeat] | 1 | 12 / 0 | Listed | 12 combinations cross fractional tile sizes, round/space repeat and the positioning area. Rounded tile counts and distributed gaps must use the selected origin. |
| [background/gradients][css/background/gradients] | 1 | 0 / 0 | Not listed | Continuous, hard-stop, repeating and radial gradients preserve stop positions and sRGB interpolation. |
| [background/layer-lists][css/background/layer-lists] | 1 | 8 / 0 | Listed | Eight combinations verify repeated and independently specified size, repeat and position lists across two background images. |
| [background/origin-clip][css/background/origin-clip] | 1 | 3 / 0 | Not listed | The same bottom-right 40x30 tile is positioned against three different origin boxes, then cropped to the content box. The border and content dimensions stay fixed. |
| [background/position-order][css/background/position-order] | 1 | 8 / 0 | Listed | Eight controls cross background-position shorthand order with independent X/Y lists of different lengths. Later declarations win on each axis and shorter lists repeat. |
| [background/rounded-position][css/background/rounded-position] | 1 | 6 / 0 | Listed | Six controls position round-resized tiles using their final size, with explicit left/center/right and top/center/bottom positions. |
| [background/text-effects][css/background/text-effects] | 1 | 0 / 0 | Listed | Eight combinations verify text-clipped repeated gradients with background positioning, blur and rotation. |
| [backgrounds/svg-image][css/backgrounds/svg-image] | 1 | 0 / 0 | Not listed | SVG background images preserve their intrinsic aspect ratio, scale to the requested size and use the same positioning, clipping and tiling rules as raster images. |
| [border/dynamic-corners][css/border/dynamic-corners] | 8 | 0 / 0 | Listed | State changes and restoration for css/border/unequal-corners. The eight states reuse the independently reviewed static combinations. |
| [border/logical-corners][css/border/logical-corners] | 1 | 0 / 0 | Listed | Six writing mode/direction combinations map four distinct logical radii to physical corners. |
| [border/mixed-joins][css/border/mixed-joins] | 1 | 0 / 0 | Listed | 16 mixed-style corner combinations cross the top edge style, unequal widths and asymmetric radii. Neighboring styles must not leave geometric holes. |
| [border/styles-radius][css/border/styles-radius] | 1 | 0 / 0 | Not listed | Solid/double/dashed/dotted borders and elliptical/asymmetric rounded corners use the declared widths and outer shapes. |
| [border/unequal-corners][css/border/unequal-corners] | 1 | 0 / 0 | Listed | 32 combinations cross all eight visible border styles, equal/unequal widths and square/asymmetric corners. Different edge colors expose joins and clipping. |
| [box-model/border-box][css/box-model/border-box] | 1 | 2 / 0 | Not listed | The 120x100 border box includes 8px of border and 20px of padding; the 80x60 content box expands to 108x88. |
| [box-model/content-box][css/box-model/content-box] | 1 | 1 / 0 | Not listed | Fixed content-box width and height exclude padding and borders, which extend outside the content box. |
| [box-model/percentage-padding][css/box-model/percentage-padding] | 1 | 2 / 0 | Not listed | Child percentage width/height use the parent content box. All four percentage paddings use its 220px inline size, so each is 22px even in the shorter vertical axis. |
| [cascade/logical-order][css/cascade/logical-order] | 1 | 12 / 0 | Listed | 12 combinations verify declaration order when logical and physical padding target the same edge in three writing modes and both directions. |
| [cascade/wide-keywords][css/cascade/wide-keywords] | 1 | 2 / 0 | Listed | CSS-wide keywords copy computed parent values, reset initial values and resolve unset by inheritance, at the declaration's place in shorthand and longhand order. |
| [color/current-mix][css/color/current-mix] | 1 | 0 / 0 | Listed | currentColor mixtures resolve on the consuming element through CSS gradients, shadows, inherited text and SVG paints, including nested mixtures and compact SVG instance rows. |
| [color/dynamic-spaces-opacity][css/color/dynamic-spaces-opacity] | 8 | 0 / 0 | Listed | State changes and restoration for css/color/spaces-opacity. The eight states reuse the independently reviewed static combinations. |
| [color/spaces-opacity][css/color/spaces-opacity] | 1 | 24 / 0 | Listed | 24 solid-region controls cross all supported color() RGB/XYZ spaces and Lab-family colors with full/half opacity in SDR output. |
| [color/srgb-alpha][css/color/srgb-alpha] | 1 | 0 / 0 | Not listed | Hex, rgb, hsl and color(srgb) produce their sRGB colors; straight-alpha black/red composite over white, including fully transparent input. |
| [compositing/backdrop-blend/basic][css/compositing/backdrop-blend/basic] | 1 | 0 / 0 | Not listed | Backdrop invert, multiply and opacity must preserve their compositing order. Controls use full, half and zero opacity; the final box has no foreground content and must not multiply the filtered page by the original page again. |
| [compositing/backdrop-blend/clips][css/compositing/backdrop-blend/clips] | 1 | 0 / 0 | Not listed | Foreground blending happens against the filtered backdrop before clip, alpha mask and group opacity. Full/half/zero opacity controls retain identical clip boundaries. |
| [compositing/backdrop-blend/controls][css/compositing/backdrop-blend/controls] | 1 | 0 / 0 | Not listed | Isolated controls for the backdrop/blend combination: invert alone filters the page before normal source-over content, while multiply alone blends foreground content with the original page. |
| [compositing/backdrop-blend/empty-bounds][css/compositing/backdrop-blend/empty-bounds] | 1 | 0 / 0 | Not listed | A zero-width backdrop has no pixels to filter but still isolates its overflowing foreground. The multiply-only control sees the blue page, while the filtered control keeps the original green. |
| [compositing/backdrop-blend/nested][css/compositing/backdrop-blend/nested] | 1 | 0 / 0 | Not listed | A filtered child reads its isolated parent background. Parent opacity is applied after the child blend; the last control also applies child opacity, so parent opacity must not be folded into the backdrop twice. |
| [compositing/backdrop-blend/overflow][css/compositing/backdrop-blend/overflow] | 1 | 0 / 0 | Not listed | An ancestor overflow clip applies after the descendant backdrop/content blend. The visible control retains the protruding filter; the hidden control cuts both backdrop and foreground at the ancestor boundary. |
| [compositing/backdrop-blend/planes][css/compositing/backdrop-blend/planes] | 1 | 0 / 0 | Not listed | A preserve-3d parent isolates its planes from the page. Unlike the 2D control, a plane over a transparent scene backdrop retains its foreground colors. Half opacity composites the complete plane result once. |
| [compositing/backdrop-roots][css/compositing/backdrop-roots] | 1 | 0 / 0 | Listed | Twelve nested backdrop controls cross root-forming properties with a second backdrop read. Transparent group content exposes whether sampling leaks through an ancestor root. |
| [compositing/blend-families][css/compositing/blend-families] | 1 | 0 / 0 | Listed | 32 controls cross every supported CSS blend mode with absent/inverted backdrop and a translucent child. Solid regions separate compositing formulas from edge antialiasing. |
| [compositing/clip-mask-matrix][css/compositing/clip-mask-matrix] | 1 | 36 / 0 | Listed | 36 combinations: 3 masks × 3 clips × 2 transforms × 2 opacities. An overflowing child distinguishes clipping the group from clipping only its background. |
| [compositing/dynamic-backdrop-roots][css/compositing/dynamic-backdrop-roots] | 8 | 0 / 0 | Listed | State changes and restoration for css/compositing/backdrop-roots. The eight states reuse the independently reviewed static combinations. |
| [compositing/dynamic-layers][css/compositing/dynamic-layers] | 8 | 1 / 0 | Listed | Eight states change opacity, filter, transform, clip, mask, display and visibility around nested backdrop/blend layers. Zero opacity, display:none and visibility:hidden must produce identical background-only frames. |
| [compositing/filter-blend-matrix][css/compositing/filter-blend-matrix] | 1 | 36 / 0 | Listed | 36 combinations: 4 backdrop filters × 3 blend modes × 3 opacities. Each filter has normal-blend and zero-opacity controls; translucent content leaves the filtered backdrop visible. |
| [compositing/group-opacity][css/compositing/group-opacity] | 1 | 0 / 0 | Not listed | Group opacity applies once after opaque children overlap; per-child alpha blends twice in the overlap and therefore gives a different color. |
| [compositing/isolation-blend][css/compositing/isolation-blend] | 1 | 0 / 0 | Not listed | Without isolation, cyan/magenta multiply with the yellow page; an isolated group blends children against its transparent group backdrop, then composites the group normally. |
| [compositing/nested-matrix][css/compositing/nested-matrix] | 1 | 32 / 0 | Listed | 32 combinations: isolation × parent opacity × overflow × empty/content × inner blend. Nested backdrop regions extend outside the parent so grouping, clipping and empty layers remain observable. |
| [compositing/rounded-blend][css/compositing/rounded-blend] | 1 | 1 / 0 | Not listed | The oversized yellow child is clipped to rounded corners and multiplied with the aqua parent inside an isolated group: lime inside, aqua in the corners, white outside the parent. |
| [compositing/scene-blend-isolation][css/compositing/scene-blend-isolation] | 1 | 0 / 0 | Not listed | A preserve-3d scene establishes an isolated stacking context. Its blending planes see the scene background and previously painted planes, not the page outside it. The two-plane overlap is blue; a transparent scene keeps cyan instead of multiplying it by the yellow page. |
| [compositing/stacking-opacity][css/compositing/stacking-opacity] | 1 | 0 / 0 | Not listed | An opacity group traps its high-z-index descendant below a later sibling stacking context. The opaque positioned control lets that descendant participate in the outer context. |
| [compositing/visibility-backdrop][css/compositing/visibility-backdrop] | 1 | 6 / 0 | Listed | Six controls distinguish visible, zero opacity, inherited hidden, display:none and explicitly visible children of a hidden filtered box, with normal and multiply blending. A hidden box must not paint its backdrop; visible descendants still paint. |
| [filter/blur-overflow][css/filter/blur-overflow] | 1 | 0 / 0 | Not listed | Blur extends beyond child geometry. The ancestor overflow clip truncates the filtered pixels at its padding box, while the visible control preserves them. |
| [filter/opacity][css/filter/opacity] | 1 | 0 / 0 | Not listed | Children composite before their parent invert filter and group opacity. A translucent child changes the overlap before filtering, rather than applying parent opacity twice. |
| [filter/order][css/filter/order] | 1 | 0 / 0 | Not listed | Filter functions apply in authored order to the complete group, including its child: invert then brightness is observably different from brightness then invert. |
| [filter/transformed-bounds][css/filter/transformed-bounds] | 1 | 18 / 0 | Listed | 18 combinations: 3 filters × 3 transforms × ancestor overflow. Blur and shadow bounds extend beyond the source and must be clipped after the child transform by the rounded ancestor. |
| [flex/alignment-order][css/flex/alignment-order] | 1 | 3 / 0 | Not listed | Order rearranges items to widths 60/80/40; 120px remaining width forms two 60px gaps. align-self overrides the centered cross-axis alignment. |
| [flex/automatic-minimum][css/flex/automatic-minimum] | 1 | 4 / 0 | Not listed | The automatic minimum keeps the first item at its 180px min-content width, overflowing the 220px row beside an 80px sibling. min-width:0 permits 140px instead. |
| [flex/column-minimum][css/flex/column-minimum] | 1 | 6 / 0 | Not listed | A column flex item keeps its 120px content minimum unless min-height: 0 is explicit. The latter shrinks to 90px after the 80px footer and 10px gap. |
| [flex/constrained-growth][css/flex/constrained-growth] | 1 | 3 / 0 | Not listed | Equal growth first proposes 100px each; freezing the first item at max-width 80px redistributes the remaining 220px as 110/110. |
| [flex/constraints-matrix][css/flex/constraints-matrix] | 1 | 72 / 0 | Listed | 24 combinations cross parent width, gap, box-sizing and row direction. Equal percentage bases shrink equally until the first item hits its min/max; the unfrozen second item takes the remaining space. |
| [flex/dynamic-constraints][css/flex/dynamic-constraints] | 8 | 1 / 0 | Listed | Widths cross the exact 120px flex wrapping threshold in both directions; a flex/grid parent alternates while nested percentage sizing remains min-constrained. |
| [flex/grid-child][css/flex/grid-child] | 1 | 7 / 0 | Not listed | A growing grid inside a padded flex row receives the remaining 280px. Its 260 by 160 content box resolves fractional tracks after gaps. |
| [flex/grow-shrink][css/flex/grow-shrink] | 1 | 4 / 0 | Not listed | After a 12px gap, 288px free width grows 1:2 to 96/192. A 300px basis shrinks to 240 proportionally to 160/80. |
| [flex/percentage-basis][css/flex/percentage-basis] | 1 | 6 / 0 | Not listed | Percentage bases resolve against the 300px inner main size before the 20px gap. Shrinking makes two 140px items; with shrink disabled both remain 150px and overflow. |
| [flex/reverse-auto-margins][css/flex/reverse-auto-margins] | 1 | 8 / 0 | Not listed | Reversed axes place the first item against the main-start edge on the right/bottom; an auto main-axis margin absorbs free width. |
| [flex/wrap-gap][css/flex/wrap-gap] | 1 | 4 / 0 | Not listed | Two 100px items and a 20px column gap fill each 220px line; the next line starts after 40px height plus 12px row gap. |
| [flex/wrap-thresholds][css/flex/wrap-thresholds] | 1 | 108 / 0 | Listed | 36 combinations cross widths just below/at/above the fit threshold, gap, wrap mode and row direction. Only the below-threshold wrapping cases gain a line; nowrap shrinks both items to 59px. |
| [generated/boxes-00][css/generated/boxes-00] | 1 | 494 / 0 | Listed | 50 fixed-seed trees (1990270464–1990270513) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-01][css/generated/boxes-01] | 1 | 484 / 0 | Listed | 50 fixed-seed trees (1990270514–1990270563) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-02][css/generated/boxes-02] | 1 | 486 / 0 | Listed | 50 fixed-seed trees (1990270564–1990270613) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-03][css/generated/boxes-03] | 1 | 488 / 0 | Listed | 50 fixed-seed trees (1990270614–1990270663) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-04][css/generated/boxes-04] | 1 | 502 / 0 | Listed | 50 fixed-seed trees (1990270664–1990270713) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-05][css/generated/boxes-05] | 1 | 488 / 0 | Listed | 50 fixed-seed trees (1990270714–1990270763) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-06][css/generated/boxes-06] | 1 | 496 / 0 | Listed | 50 fixed-seed trees (1990270764–1990270813) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-07][css/generated/boxes-07] | 1 | 480 / 0 | Listed | 50 fixed-seed trees (1990270814–1990270863) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-08][css/generated/boxes-08] | 1 | 494 / 0 | Listed | 50 fixed-seed trees (1990270864–1990270913) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-09][css/generated/boxes-09] | 1 | 474 / 0 | Listed | 50 fixed-seed trees (1990270914–1990270963) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-10][css/generated/boxes-10] | 1 | 476 / 0 | Listed | 50 fixed-seed trees (1990270964–1990271013) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-11][css/generated/boxes-11] | 1 | 500 / 0 | Listed | 50 fixed-seed trees (1990271014–1990271063) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-12][css/generated/boxes-12] | 1 | 496 / 0 | Listed | 50 fixed-seed trees (1990271064–1990271113) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-13][css/generated/boxes-13] | 1 | 490 / 0 | Listed | 50 fixed-seed trees (1990271114–1990271163) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-14][css/generated/boxes-14] | 1 | 484 / 0 | Listed | 50 fixed-seed trees (1990271164–1990271213) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-15][css/generated/boxes-15] | 1 | 480 / 0 | Listed | 50 fixed-seed trees (1990271214–1990271263) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-16][css/generated/boxes-16] | 1 | 482 / 0 | Listed | 50 fixed-seed trees (1990271264–1990271313) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-17][css/generated/boxes-17] | 1 | 486 / 0 | Listed | 50 fixed-seed trees (1990271314–1990271363) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-18][css/generated/boxes-18] | 1 | 472 / 0 | Listed | 50 fixed-seed trees (1990271364–1990271413) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [generated/boxes-19][css/generated/boxes-19] | 1 | 490 / 0 | Listed | 50 fixed-seed trees (1990271414–1990271463) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model. |
| [grid/auto-placement-dense][css/grid/auto-placement-dense] | 1 | 6 / 0 | Not listed | Sparse row placement leaves the first-row hole after two-column items; dense placement puts the final one-column item back into that hole. |
| [grid/auto-repeat][css/grid/auto-repeat] | 1 | 4 / 0 | Not listed | auto-fill retains three 100px tracks after gaps; auto-fit collapses the unoccupied third track and gap, producing two 155px tracks. |
| [grid/flex-child][css/grid/flex-child] | 1 | 7 / 0 | Not listed | Explicit grid tracks become the containing width for nested flex layout. Padding and gaps are deducted once; cross-axis centering is relative to each cell. |
| [grid/named-areas][css/grid/named-areas] | 1 | 4 / 0 | Not listed | Named areas span the authored cells: head and foot span both columns; side and main occupy the middle row. |
| [grid/named-lines-implicit][css/grid/named-lines-implicit] | 1 | 2 / 0 | Not listed | Named column lines delimit a 190px span. Placement in row 3 creates implicit rows of 30px then 50px, separated by 10px gaps. |
| [grid/placement][css/grid/placement] | 1 | 3 / 0 | Not listed | Explicit grid lines place a two-column span and a two-row span; self alignment centers a 40x20 item inside an 80x50 cell. |
| [grid/tracks][css/grid/tracks] | 1 | 6 / 0 | Not listed | After 80px fixed track and 20px gaps, 240px is split into 80px and 160px fractional tracks. Explicit rows retain 40px and 60px heights. |
| [images/dynamic-fit-extremes][css/images/dynamic-fit-extremes] | 8 | 0 / 0 | Listed | State changes and restoration for css/images/fit-extremes. The eight states reuse the independently reviewed static combinations. |
| [images/fit-extremes][css/images/fit-extremes] | 1 | 8 / 0 | Listed | Eight combinations verify none/scale-down with boxes smaller and larger than the image and opposite object positions. Scale-down never enlarges the intrinsic image. |
| [images/object-fit][css/images/object-fit] | 1 | 3 / 0 | Not listed | A 2:1 image is contained, covered or filled in a 100x100 box. Integer scaling and pixelated sampling expose fit/crop boundaries without interpolation noise. |
| [images/object-position][css/images/object-position] | 1 | 3 / 0 | Not listed | A 2:1 image covers a square, cropping horizontally. Left, center and right object positions select different 2-color windows of the 4-color source. |
| [layout/block-flow][css/layout/block-flow] | 1 | 4 / 0 | Not listed | Block children fill the 256px content width, stack vertically and split horizontal auto margins equally. |
| [layout/logical-sides-matrix][css/layout/logical-sides-matrix] | 1 | 12 / 0 | Listed | Six writing mode and direction combinations map inset-inline-start, inset-block-start, inline-size, block-size and the four flow-relative margins, paddings and borders, each with a different value, to their physical sides and axes. |
| [layout/nested-sizing][css/layout/nested-sizing] | 1 | 32 / 0 | Listed | Eight combinations cross outer/inner Flex/Grid with definite percentage constraints or max-content sizing. The inner content is 50+10+70=130px; its border box is 146px and the intrinsic outer box is 158px. |
| [layout/vertical-block][css/layout/vertical-block] | 1 | 22 / 0 | Listed | Vertical block flow, auto inline stretch, logical margins, orthogonal intrinsic sizing and opposite nested block directions. |
| [layout/vertical-flex][css/layout/vertical-flex] | 1 | 20 / 0 | Listed | Flex main/cross axes follow writing-mode, including row/column reverse, wrapping, gaps and RTL inline progression. |
| [layout/vertical-grid][css/layout/vertical-grid] | 1 | 20 / 0 | Listed | Grid columns follow the inline axis and rows the block axis, with spanning, fractions, gaps, RTL and an orthogonal aspect-ratio item. |
| [layout/vertical-position][css/layout/vertical-position] | 1 | 20 / 0 | Listed | Physical and logical absolute insets, percentage padding and static-position hoisting through an orthogonal flow. |
| [mask/backdrop][css/mask/backdrop] | 1 | 0 / 0 | Not listed | A clip and binary alpha mask limit both the filtered backdrop and foreground content; the control at half opacity reveals the unfiltered page beneath. |
| [mask/clip][css/mask/clip] | 1 | 0 / 0 | Not listed | Clip and alpha mask intersect over the whole group before group opacity. The right half is removed even where a child extends beyond the remaining area. |
| [mask/composite][css/mask/composite] | 1 | 0 / 0 | Not listed | Two binary alpha masks combine by add, subtract, intersect or exclude. The first mask is the source and the lower mask the destination; compositing changes the child with its parent. |
| [mask/dynamic-three-layers][css/mask/dynamic-three-layers] | 8 | 0 / 0 | Listed | State changes and restoration for css/mask/three-layer-composites. The eight states reuse the independently reviewed static combinations. |
| [mask/shape][css/mask/shape] | 1 | 5 / 0 | Listed | CSS shape() resolves responsive lengths, relative and absolute curves, reflected controls, arcs and evenodd subpaths in the selected reference box before transforms. |
| [mask/three-layer-composites][css/mask/three-layer-composites] | 1 | 0 / 0 | Listed | Eight controls cross all four mask-composite operators with alpha/luminance for three asymmetric layers, exposing list repetition, composition order and mask interpretation. |
| [overflow/clipping][css/overflow/clipping] | 1 | 2 / 0 | Not listed | The first oversized child is clipped to its 120px padding box; the second extends 20px outside both edges. |
| [position/absolute-padding][css/position/absolute-padding] | 1 | 3 / 0 | Not listed | A static intermediate box does not become the containing block. Absolute percentage sizes and right/bottom offsets use the positioned ancestor padding box (232 by 152). |
| [position/absolute-relative][css/position/absolute-relative] | 1 | 3 / 0 | Not listed | Absolute insets use the parent padding box and do not consume flow space; relative offsets move painting without moving the following block. |
| [position/logical-containing-block][css/position/logical-containing-block] | 1 | 24 / 0 | Listed | 12 combinations cross writing mode, direction and absolute/fixed positioning. An identity transform establishes the containing block through a static intermediate; logical insets resolve against its padding box. |
| [shadow/blur][css/shadow/blur] | 1 | 0 / 0 | Not listed | Outer and inset shadows blur an offset silhouette while retaining the solid box and clipping the inset shadow to its interior. |
| [shadow/dynamic-outline][css/shadow/dynamic-outline] | 8 | 0 / 0 | Listed | State changes and restoration for css/shadow/outline-layers. The eight states reuse the independently reviewed static combinations. |
| [shadow/outer-inset][css/shadow/outer-inset] | 1 | 0 / 0 | Not listed | An 8x10 offset and 4px spread shadow expands outside one box and clips inside the other; zero blur makes boundaries exact. |
| [shadow/outline-layers][css/shadow/outline-layers] | 1 | 12 / 0 | Listed | 12 combinations cross clipping, rotation and multiple inner/outer shadows with an offset outline. Rounded clipping applies to the complete transformed child. |
| [shadow/rounded-clip][css/shadow/rounded-clip] | 1 | 0 / 0 | Not listed | A rounded overflow clip contains a translated child, including its opacity group, while the parent outer shadow remains outside the clip. |
| [sizing/aspect-ratio][css/sizing/aspect-ratio] | 1 | 2 / 0 | Not listed | A definite width determines auto height at 2:1, and a definite height determines auto width at 1:1. |
| [sizing/auto-height-limits][css/sizing/auto-height-limits] | 1 | 3 / 0 | Not listed | Auto height is 20+30+16+4=70px. min-height raises it to 100px; max-height lowers it to 48px and overflow clips the children. |
| [sizing/dynamic-percentages][css/sizing/dynamic-percentages] | 8 | 0 / 0 | Listed | State changes and restoration for css/sizing/indefinite-percentages. The eight states reuse the independently reviewed static combinations. |
| [sizing/indefinite-percentages][css/sizing/indefinite-percentages] | 1 | 24 / 0 | Listed | 12 combinations cross Block/Flex/Grid, definite/indefinite parent height and child minimum height. Percentage width clamps to 90px; cyclic percentage height resolves through intrinsic content. |
| [sizing/intrinsic-constraints][css/sizing/intrinsic-constraints] | 1 | 12 / 0 | Listed | 12 combinations cross intrinsic size keywords, maximum size and row/column layout. Controls expose clamping and wrapping independently on both physical axes. |
| [sizing/intrinsic-width][css/sizing/intrinsic-width] | 1 | 2 / 0 | Not listed | A wrapping flex container has min-content width 100px (widest item) and max-content width 170px (60+10+100). At the minimum it has two 30px lines and one 10px gap. |
| [sizing/percentage-min-max][css/sizing/percentage-min-max] | 1 | 4 / 0 | Not listed | Percentages resolve against the 320px containing block, then max/min clamp 240 to 200 and 80 to 120. calc(50% - 20px) is 140px. |
| [text/alignment][css/text/alignment] | 1 | 1 / 1 | Not listed | Two preformatted lines align their right edge to x=260, while baselines remain separated by 32px. |
| [text/alignment-matrix][css/text/alignment-matrix] | 1 | 12 / 0 | Listed | 12 independent display/alignment combinations place empty atomic inline boxes beside nested smaller superscript text; display mode is not coupled to alignment choice. |
| [text/axes-synthesis][css/text/axes-synthesis] | 1 | 0 / 0 | Listed | Eight combinations cross variable axes, synthesis permission and x-height adjustment through a smaller/bolder nested run. Native variable axes remain active when synthesis is disabled. |
| [text/bidi][css/text/bidi] | 1 | 0 / 0 | Listed | Direction sets paragraph alignment, mixed-script ordering, flex order and logical box sides. CSS embedding, overrides, isolates and plaintext preserve source indices and scope boundaries. |
| [text/bidi-inline-atoms][css/text/bidi-inline-atoms] | 1 | 0 / 0 | Listed | 18 combinations cross bidi direction, writing mode and atomic inline alignment with explicitly declared Latin, Hebrew and Han fonts and a decorated smaller run. |
| [text/breaking-products][css/text/breaking-products] | 1 | 0 / 0 | Listed | 108 combinations exhaust the selected white-space × word-break × overflow-wrap values at two nearby widths, with spaces, an explicit newline, a long word and Han text. |
| [text/caps][css/text/caps] | 1 | 0 / 0 | Listed | Petite/all-petite/unicase use native small-cap fallbacks where present. Each row matches its explicit OpenType-feature control. Synthetic casing, caseless characters and byte indices are covered by text unit tests. |
| [text/combine][css/text/combine] | 1 | 0 / 0 | Listed | Combined text remains horizontal in one vertical 1em cell, including fullwidth digits, compression, both column directions, underline and one emphasis mark for the whole cell. |
| [text/decoration-cjk][css/text/decoration-cjk] | 1 | 10 / 10 | Not listed | Auto ink skipping keeps CJK glyphs and punctuation continuously decorated, while Latin descenders still make gaps. Solid, double, dotted, dashed and wavy underlines share this rule; an explicit offset moves the line without changing text layout. Transparent glyph fill isolates the decoration pixels without removing the glyph ink used to compute gaps. |
| [text/decoration-matrix][css/text/decoration-matrix] | 1 | 30 / 0 | Listed | 30 combinations cross five underline styles, three writing modes and two group opacities on mixed-color text with centered stroke and a sharp shadow. |
| [text/dynamic-axes][css/text/dynamic-axes] | 8 | 0 / 0 | Listed | State changes and restoration for css/text/axes-synthesis. The eight states reuse the independently reviewed static combinations. |
| [text/dynamic-bidi-atoms][css/text/dynamic-bidi-atoms] | 8 | 0 / 0 | Listed | State changes and restoration for css/text/bidi-inline-atoms. The eight states reuse the independently reviewed static combinations. |
| [text/dynamic-breaking][css/text/dynamic-breaking] | 8 | 0 / 0 | Listed | State changes and restoration for css/text/breaking-products. The eight states reuse the independently reviewed static combinations. |
| [text/dynamic-inheritance][css/text/dynamic-inheritance] | 8 | 1 / 0 | Listed | Inherited font size, line height and color change alongside the wrap threshold, RTL ellipsis and mixed-size vertical writing. Repeated seek restores glyph positions, line counts and paragraph direction. |
| [text/dynamic-nested-decorations][css/text/dynamic-nested-decorations] | 8 | 0 / 0 | Listed | State changes and restoration for css/text/nested-decorations. The eight states reuse the independently reviewed static combinations. |
| [text/ellipsis][css/text/ellipsis] | 1 | 2 / 0 | Not listed | A non-wrapping overflowing line is either clipped or ends in an ellipsis; neither mode draws outside its padding box. |
| [text/emphasis][css/text/emphasis] | 1 | 0 / 0 | Listed | Emphasis marks are centered on typographic units, skip spaces and punctuation, honor custom glyphs, colors and horizontal/vertical placement. |
| [text/font-features][css/text/font-features] | 1 | 0 / 0 | Listed | 44 keyword controls exercise kerning and every accepted font-variant longhand value. Each selected feature is paired with the same font and text at its default setting. |
| [text/font-position][css/text/font-position] | 1 | 4 / 0 | Listed | Native numeral variants remain native; incomplete runs synthesize all glyphs without changing line metrics, including adjusted sizes and caps. |
| [text/font-shorthand][css/text/font-shorthand] | 1 | 2 / 0 | Not listed | Font shorthand assigns weight, size, line height and family and resets omitted italic and weight declarations. |
| [text/font-size-adjust][css/text/font-size-adjust] | 1 | 1 / 0 | Listed | Adjusted 24px Noto Sans with aspect 1.072 matches a 48px control, including a Span override. The box uses 31.25px computed size: em stays 31.25px while ex becomes 33.5px. |
| [text/font-size-keywords][css/text/font-size-keywords] | 1 | 8 / 0 | Listed | A fixed 16px medium size table resolves em widths; smaller in the font shorthand resolves against the parent computed size. |
| [text/font-synthesis][css/text/font-synthesis] | 1 | 0 / 0 | Listed | A regular-only font demonstrates weight/style permissions independently. none disables synthesis, and weight does not allow a synthetic italic. |
| [text/inline-box][css/text/inline-box] | 1 | 3 / 0 | Listed | Inline padding and borders participate in line breaking; backgrounds and borders slice continuously across wrapped fragments, including nested and empty spans. |
| [text/inline-boxes][css/text/inline-boxes] | 1 | 3 / 0 | Listed | Replaced content and inline block, flex and grid boxes wrap with text, use their margins and appropriate baselines, and retain nested content and Span alignment. |
| [text/inline-directions][css/text/inline-directions] | 1 | 2 / 0 | Listed | Inline decorations follow RTL order and the vertical inline axis, preserving sliced edges and the physical gradient direction on wrapped spans. |
| [text/inline-writing][css/text/inline-writing] | 1 | 3 / 0 | Listed | Inline boxes retain independent writing modes while their margin boxes participate in vertical and RTL paragraphs, including nested inline alignment and backgrounds. |
| [text/mixed-runs][css/text/mixed-runs] | 1 | 1 / 0 | Not listed | Normal, bold larger and italic runs share one baseline; their sizes and shapes remain distinct inside a 48px line. |
| [text/mixed-size-matrix][css/text/mixed-size-matrix] | 1 | 12 / 0 | Listed | 12 combinations cross three writing modes, two orientations and two child sizes. Mixed vertical text aligns on the central baseline; sideways retains alphabetic alignment. |
| [text/nested-decorations][css/text/nested-decorations] | 1 | 0 / 0 | Listed | Four nested decoration controls combine wrapping, mixed sizes, propagated outer decorations and an independently colored inner double underline. |
| [text/overflow-wrap][css/text/overflow-wrap] | 1 | 1 / 1 | Not listed | overflow-wrap:anywhere breaks an otherwise unbreakable word into two four-glyph lines inside a 100px content width. |
| [text/paint-order][css/text/paint-order] | 1 | 1 / 1 | Not listed | Text inherits paint-order. Painting the centered stroke before the opaque fill preserves the interiors of CJK and Latin glyphs without changing line layout. |
| [text/paragraph][css/text/paragraph] | 1 | 5 / 5 | Listed | Last-line alignment applies at soft and forced paragraph ends; pretty avoids a final orphan without adding a line; anywhere breaks even no-break spaces and keep-all; strict and loose differ at small kana. |
| [text/preserved-spaces][css/text/preserved-spaces] | 1 | 1 / 1 | Not listed | white-space: pre preserves doubled spaces and the explicit line break, with a 32px line height and the same pinned Noto Sans font in every renderer. |
| [text/relative-inheritance][css/text/relative-inheritance] | 1 | 0 / 0 | Listed | Eight parent size/weight controls exercise larger/smaller and bolder/lighter inheritance in adjacent runs and at weight-range boundaries. |
| [text/spacing-indent][css/text/spacing-indent] | 1 | 1 / 1 | Not listed | Letter spacing adds 2px per glyph slot and word spacing adds 4px to the preserved space. The 20px indent applies to the first line only. |
| [text/truncation-matrix][css/text/truncation-matrix] | 1 | 12 / 0 | Listed | Eight width/overflow/direction combinations check clip versus ellipsis; four font-size/line-clamp combinations check one and two visible lines with independently expected 32px and 64px auto heights. |
| [text/underline-position][css/text/underline-position] | 1 | 0 / 0 | Listed | Underline position places horizontal lines below descenders and vertical lines on the chosen physical side, with inherited values, mixed sizes and explicit offsets. |
| [text/vertical][css/text/vertical] | 1 | 4 / 0 | Listed | Vertical Text lays out columns in both directions, uses upright CJK and vertical punctuation with sideways Latin, supports upright and sideways overrides, and preserves styled text and decorations. |
| [text/vertical-align][css/text/vertical-align] | 1 | 12 / 0 | Listed | Inline alignment positions child line boxes relative to the parent or whole line, with nested scopes, percentages and signed lengths. Span backgrounds follow their own baselines. |
| [text/vertical-container][css/text/vertical-container] | 1 | 8 / 0 | Listed | Inherited vertical text and percentage inline padding coexist with a horizontal paragraph and an aspect-ratio SVG in vertical flex containers. |
| [text/word-wrap][css/text/word-wrap] | 1 | 1 / 1 | Not listed | Normal white-space wraps at the word boundary after beta; words remain whole and baselines are 32px apart. |
| [text/wrap-matrix][css/text/wrap-matrix] | 1 | 24 / 24 | Listed | 24 combinations cross width around four M advances, white-space, word-break and overflow-wrap. Noto Sans M advances 907/1000 em: 86px fits three at 24px, 88px fits four; nowrap disables emergency wrapping. |
| [text/wrapping][css/text/wrapping] | 1 | 1 / 1 | Not listed | At 24px Noto Sans, four M glyphs fit in 100px and five do not. break-all wraps twelve glyphs into three lines at a 32px line height. |
| [transform/clip][css/transform/clip] | 1 | 0 / 0 | Not listed | A child clip rotates with its element while the ancestor overflow clip stays in ancestor space. Nested child content obeys both clips. |
| [transform/dynamic-order-clipping][css/transform/dynamic-order-clipping] | 8 | 0 / 0 | Listed | State changes and restoration for css/transform/order-clipping. The eight states reuse the independently reviewed static combinations. |
| [transform/dynamic-perspective][css/transform/dynamic-perspective] | 8 | 0 / 0 | Listed | State changes and restoration for css/transform/perspective-groups. The eight states reuse the independently reviewed static combinations. |
| [transform/order-clipping][css/transform/order-clipping] | 1 | 0 / 0 | Listed | Twelve transform controls cross three origins, noncommutative transform lists and rounded ancestor clipping. Individual translate/rotate/scale precede the transform list; negative scale reverses handedness. |
| [transform/origin-order][css/transform/origin-order] | 1 | 0 / 0 | Not listed | Translation moves a box by 20/40px; top-left scaling doubles both axes; a 90-degree rotation swaps extents around the top-left origin. |
| [transform/perspective-groups][css/transform/perspective-groups] | 1 | 0 / 0 | Listed | Twelve perspective controls cross perspective origin, preserve-3d/flat and an overflow grouping property. Nested Z translations expose flattening and projection order. |
| [transforms/dynamic-path-extents][css/transforms/dynamic-path-extents] | 8 | 0 / 0 | Listed | State changes and restoration for css/transforms/path-extents. The eight states reuse the independently reviewed static combinations. |
| [transforms/path-extents][css/transforms/path-extents] | 1 | 0 / 0 | Listed | Sixteen open-path controls cross before/start/middle/after distances, anchor and tangent direction. Out-of-range distance clamps to endpoints while reverse flips orientation. |
| [transforms/shape-path][css/transforms/shape-path] | 1 | 2 / 0 | Listed | Responsive shape motion paths use the containing reference box, follow curve tangents and clamp open ends. |
| [values/absolute-units][css/values/absolute-units] | 1 | 6 / 0 | Listed | Absolute units resolve to CSS reference pixels, including mixed-unit calc; each left bar equals its independently sized pixel control. |
| [values/dynamic-inherited-units][css/values/dynamic-inherited-units] | 8 | 0 / 0 | Listed | State changes and restoration for css/values/inherited-units. The eight states reuse the independently reviewed static combinations. |
| [values/font-ex][css/values/font-ex] | 1 | 4 / 0 | Listed | ex uses the selected font x-height. Pinned Noto Sans has 536/1000 units/em at weight 400 and 546/1000 at weight 700. Checks calc, inherited font size and font variation against pixel controls. |
| [values/inherited-units][css/values/inherited-units] | 1 | 48 / 0 | Listed | 24 combinations cross inherited font size, em/rem/px/percentage lengths and writing mode. Physical percentage width remains relative to the 160px containing width. |

[css/background/gradients]: ../tests/cases/css/background/gradients/case.json
[css/background/origin-clip]: ../tests/cases/css/background/origin-clip/case.json
[css/backgrounds/svg-image]: ../tests/cases/css/backgrounds/svg-image/case.json
[css/border/styles-radius]: ../tests/cases/css/border/styles-radius/case.json
[css/box-model/border-box]: ../tests/cases/css/box-model/border-box/case.json
[css/box-model/content-box]: ../tests/cases/css/box-model/content-box/case.json
[css/box-model/percentage-padding]: ../tests/cases/css/box-model/percentage-padding/case.json
[css/cascade/wide-keywords]: ../tests/cases/css/cascade/wide-keywords/case.json
[css/color/current-mix]: ../tests/cases/css/color/current-mix/case.json
[css/color/srgb-alpha]: ../tests/cases/css/color/srgb-alpha/case.json
[css/compositing/backdrop-blend/basic]: ../tests/cases/css/compositing/backdrop-blend/basic/case.json
[css/compositing/backdrop-blend/clips]: ../tests/cases/css/compositing/backdrop-blend/clips/case.json
[css/compositing/backdrop-blend/controls]: ../tests/cases/css/compositing/backdrop-blend/controls/case.json
[css/compositing/backdrop-blend/empty-bounds]: ../tests/cases/css/compositing/backdrop-blend/empty-bounds/case.json
[css/compositing/backdrop-blend/nested]: ../tests/cases/css/compositing/backdrop-blend/nested/case.json
[css/compositing/backdrop-blend/overflow]: ../tests/cases/css/compositing/backdrop-blend/overflow/case.json
[css/compositing/backdrop-blend/planes]: ../tests/cases/css/compositing/backdrop-blend/planes/case.json
[css/compositing/clip-mask-matrix]: ../tests/cases/css/compositing/clip-mask-matrix/case.json
[css/compositing/dynamic-layers]: ../tests/cases/css/compositing/dynamic-layers/case.json
[css/compositing/filter-blend-matrix]: ../tests/cases/css/compositing/filter-blend-matrix/case.json
[css/compositing/group-opacity]: ../tests/cases/css/compositing/group-opacity/case.json
[css/compositing/isolation-blend]: ../tests/cases/css/compositing/isolation-blend/case.json
[css/compositing/nested-matrix]: ../tests/cases/css/compositing/nested-matrix/case.json
[css/compositing/rounded-blend]: ../tests/cases/css/compositing/rounded-blend/case.json
[css/compositing/scene-blend-isolation]: ../tests/cases/css/compositing/scene-blend-isolation/case.json
[css/compositing/stacking-opacity]: ../tests/cases/css/compositing/stacking-opacity/case.json
[css/compositing/visibility-backdrop]: ../tests/cases/css/compositing/visibility-backdrop/case.json
[css/filter/blur-overflow]: ../tests/cases/css/filter/blur-overflow/case.json
[css/filter/opacity]: ../tests/cases/css/filter/opacity/case.json
[css/filter/order]: ../tests/cases/css/filter/order/case.json
[css/filter/transformed-bounds]: ../tests/cases/css/filter/transformed-bounds/case.json
[css/flex/alignment-order]: ../tests/cases/css/flex/alignment-order/case.json
[css/flex/automatic-minimum]: ../tests/cases/css/flex/automatic-minimum/case.json
[css/flex/column-minimum]: ../tests/cases/css/flex/column-minimum/case.json
[css/flex/constrained-growth]: ../tests/cases/css/flex/constrained-growth/case.json
[css/flex/constraints-matrix]: ../tests/cases/css/flex/constraints-matrix/case.json
[css/flex/dynamic-constraints]: ../tests/cases/css/flex/dynamic-constraints/case.json
[css/flex/grid-child]: ../tests/cases/css/flex/grid-child/case.json
[css/flex/grow-shrink]: ../tests/cases/css/flex/grow-shrink/case.json
[css/flex/percentage-basis]: ../tests/cases/css/flex/percentage-basis/case.json
[css/flex/reverse-auto-margins]: ../tests/cases/css/flex/reverse-auto-margins/case.json
[css/flex/wrap-gap]: ../tests/cases/css/flex/wrap-gap/case.json
[css/flex/wrap-thresholds]: ../tests/cases/css/flex/wrap-thresholds/case.json
[css/grid/auto-placement-dense]: ../tests/cases/css/grid/auto-placement-dense/case.json
[css/grid/auto-repeat]: ../tests/cases/css/grid/auto-repeat/case.json
[css/grid/flex-child]: ../tests/cases/css/grid/flex-child/case.json
[css/grid/named-areas]: ../tests/cases/css/grid/named-areas/case.json
[css/grid/named-lines-implicit]: ../tests/cases/css/grid/named-lines-implicit/case.json
[css/grid/placement]: ../tests/cases/css/grid/placement/case.json
[css/grid/tracks]: ../tests/cases/css/grid/tracks/case.json
[css/images/object-fit]: ../tests/cases/css/images/object-fit/case.json
[css/images/object-position]: ../tests/cases/css/images/object-position/case.json
[css/layout/block-flow]: ../tests/cases/css/layout/block-flow/case.json
[css/layout/logical-sides-matrix]: ../tests/cases/css/layout/logical-sides-matrix/case.json
[css/layout/nested-sizing]: ../tests/cases/css/layout/nested-sizing/case.json
[css/layout/vertical-block]: ../tests/cases/css/layout/vertical-block/case.json
[css/layout/vertical-flex]: ../tests/cases/css/layout/vertical-flex/case.json
[css/layout/vertical-grid]: ../tests/cases/css/layout/vertical-grid/case.json
[css/layout/vertical-position]: ../tests/cases/css/layout/vertical-position/case.json
[css/mask/backdrop]: ../tests/cases/css/mask/backdrop/case.json
[css/mask/clip]: ../tests/cases/css/mask/clip/case.json
[css/mask/composite]: ../tests/cases/css/mask/composite/case.json
[css/mask/shape]: ../tests/cases/css/mask/shape/case.json
[css/overflow/clipping]: ../tests/cases/css/overflow/clipping/case.json
[css/position/absolute-padding]: ../tests/cases/css/position/absolute-padding/case.json
[css/position/absolute-relative]: ../tests/cases/css/position/absolute-relative/case.json
[css/position/logical-containing-block]: ../tests/cases/css/position/logical-containing-block/case.json
[css/shadow/blur]: ../tests/cases/css/shadow/blur/case.json
[css/shadow/outer-inset]: ../tests/cases/css/shadow/outer-inset/case.json
[css/shadow/rounded-clip]: ../tests/cases/css/shadow/rounded-clip/case.json
[css/sizing/aspect-ratio]: ../tests/cases/css/sizing/aspect-ratio/case.json
[css/sizing/auto-height-limits]: ../tests/cases/css/sizing/auto-height-limits/case.json
[css/sizing/intrinsic-width]: ../tests/cases/css/sizing/intrinsic-width/case.json
[css/sizing/percentage-min-max]: ../tests/cases/css/sizing/percentage-min-max/case.json
[css/text/alignment]: ../tests/cases/css/text/alignment/case.json
[css/text/alignment-matrix]: ../tests/cases/css/text/alignment-matrix/case.json
[css/text/bidi]: ../tests/cases/css/text/bidi/case.json
[css/text/caps]: ../tests/cases/css/text/caps/case.json
[css/text/combine]: ../tests/cases/css/text/combine/case.json
[css/text/decoration-cjk]: ../tests/cases/css/text/decoration-cjk/case.json
[css/text/decoration-matrix]: ../tests/cases/css/text/decoration-matrix/case.json
[css/text/dynamic-inheritance]: ../tests/cases/css/text/dynamic-inheritance/case.json
[css/text/ellipsis]: ../tests/cases/css/text/ellipsis/case.json
[css/text/emphasis]: ../tests/cases/css/text/emphasis/case.json
[css/text/font-position]: ../tests/cases/css/text/font-position/case.json
[css/text/font-shorthand]: ../tests/cases/css/text/font-shorthand/case.json
[css/text/font-size-adjust]: ../tests/cases/css/text/font-size-adjust/case.json
[css/text/font-size-keywords]: ../tests/cases/css/text/font-size-keywords/case.json
[css/text/font-synthesis]: ../tests/cases/css/text/font-synthesis/case.json
[css/text/inline-box]: ../tests/cases/css/text/inline-box/case.json
[css/text/inline-boxes]: ../tests/cases/css/text/inline-boxes/case.json
[css/text/inline-directions]: ../tests/cases/css/text/inline-directions/case.json
[css/text/inline-writing]: ../tests/cases/css/text/inline-writing/case.json
[css/text/mixed-runs]: ../tests/cases/css/text/mixed-runs/case.json
[css/text/mixed-size-matrix]: ../tests/cases/css/text/mixed-size-matrix/case.json
[css/text/overflow-wrap]: ../tests/cases/css/text/overflow-wrap/case.json
[css/text/paint-order]: ../tests/cases/css/text/paint-order/case.json
[css/text/paragraph]: ../tests/cases/css/text/paragraph/case.json
[css/text/preserved-spaces]: ../tests/cases/css/text/preserved-spaces/case.json
[css/text/spacing-indent]: ../tests/cases/css/text/spacing-indent/case.json
[css/text/truncation-matrix]: ../tests/cases/css/text/truncation-matrix/case.json
[css/text/underline-position]: ../tests/cases/css/text/underline-position/case.json
[css/text/vertical]: ../tests/cases/css/text/vertical/case.json
[css/text/vertical-align]: ../tests/cases/css/text/vertical-align/case.json
[css/text/vertical-container]: ../tests/cases/css/text/vertical-container/case.json
[css/text/word-wrap]: ../tests/cases/css/text/word-wrap/case.json
[css/text/wrap-matrix]: ../tests/cases/css/text/wrap-matrix/case.json
[css/text/wrapping]: ../tests/cases/css/text/wrapping/case.json
[css/transform/clip]: ../tests/cases/css/transform/clip/case.json
[css/transform/origin-order]: ../tests/cases/css/transform/origin-order/case.json
[css/transforms/shape-path]: ../tests/cases/css/transforms/shape-path/case.json
[css/values/absolute-units]: ../tests/cases/css/values/absolute-units/case.json
[css/values/font-ex]: ../tests/cases/css/values/font-ex/case.json

[css/background/dynamic-layer-lists]: ../tests/cases/css/background/dynamic-layer-lists/case.json

[css/background/dynamic-rounded-position]: ../tests/cases/css/background/dynamic-rounded-position/case.json

[css/background/fractional-repeat]: ../tests/cases/css/background/fractional-repeat/case.json

[css/background/layer-lists]: ../tests/cases/css/background/layer-lists/case.json

[css/background/position-order]: ../tests/cases/css/background/position-order/case.json

[css/background/rounded-position]: ../tests/cases/css/background/rounded-position/case.json

[css/background/text-effects]: ../tests/cases/css/background/text-effects/case.json

[css/border/dynamic-corners]: ../tests/cases/css/border/dynamic-corners/case.json

[css/border/logical-corners]: ../tests/cases/css/border/logical-corners/case.json

[css/border/mixed-joins]: ../tests/cases/css/border/mixed-joins/case.json

[css/border/unequal-corners]: ../tests/cases/css/border/unequal-corners/case.json

[css/cascade/logical-order]: ../tests/cases/css/cascade/logical-order/case.json

[css/color/dynamic-spaces-opacity]: ../tests/cases/css/color/dynamic-spaces-opacity/case.json

[css/color/spaces-opacity]: ../tests/cases/css/color/spaces-opacity/case.json

[css/compositing/backdrop-roots]: ../tests/cases/css/compositing/backdrop-roots/case.json

[css/compositing/blend-families]: ../tests/cases/css/compositing/blend-families/case.json

[css/compositing/dynamic-backdrop-roots]: ../tests/cases/css/compositing/dynamic-backdrop-roots/case.json

[css/generated/boxes-00]: ../tests/cases/css/generated/boxes-00/case.json

[css/generated/boxes-01]: ../tests/cases/css/generated/boxes-01/case.json

[css/generated/boxes-02]: ../tests/cases/css/generated/boxes-02/case.json

[css/generated/boxes-03]: ../tests/cases/css/generated/boxes-03/case.json

[css/generated/boxes-04]: ../tests/cases/css/generated/boxes-04/case.json

[css/generated/boxes-05]: ../tests/cases/css/generated/boxes-05/case.json

[css/generated/boxes-06]: ../tests/cases/css/generated/boxes-06/case.json

[css/generated/boxes-07]: ../tests/cases/css/generated/boxes-07/case.json

[css/generated/boxes-08]: ../tests/cases/css/generated/boxes-08/case.json

[css/generated/boxes-09]: ../tests/cases/css/generated/boxes-09/case.json

[css/generated/boxes-10]: ../tests/cases/css/generated/boxes-10/case.json

[css/generated/boxes-11]: ../tests/cases/css/generated/boxes-11/case.json

[css/generated/boxes-12]: ../tests/cases/css/generated/boxes-12/case.json

[css/generated/boxes-13]: ../tests/cases/css/generated/boxes-13/case.json

[css/generated/boxes-14]: ../tests/cases/css/generated/boxes-14/case.json

[css/generated/boxes-15]: ../tests/cases/css/generated/boxes-15/case.json

[css/generated/boxes-16]: ../tests/cases/css/generated/boxes-16/case.json

[css/generated/boxes-17]: ../tests/cases/css/generated/boxes-17/case.json

[css/generated/boxes-18]: ../tests/cases/css/generated/boxes-18/case.json

[css/generated/boxes-19]: ../tests/cases/css/generated/boxes-19/case.json

[css/images/dynamic-fit-extremes]: ../tests/cases/css/images/dynamic-fit-extremes/case.json

[css/images/fit-extremes]: ../tests/cases/css/images/fit-extremes/case.json

[css/mask/dynamic-three-layers]: ../tests/cases/css/mask/dynamic-three-layers/case.json

[css/mask/three-layer-composites]: ../tests/cases/css/mask/three-layer-composites/case.json

[css/shadow/dynamic-outline]: ../tests/cases/css/shadow/dynamic-outline/case.json

[css/shadow/outline-layers]: ../tests/cases/css/shadow/outline-layers/case.json

[css/sizing/dynamic-percentages]: ../tests/cases/css/sizing/dynamic-percentages/case.json

[css/sizing/indefinite-percentages]: ../tests/cases/css/sizing/indefinite-percentages/case.json

[css/sizing/intrinsic-constraints]: ../tests/cases/css/sizing/intrinsic-constraints/case.json

[css/text/axes-synthesis]: ../tests/cases/css/text/axes-synthesis/case.json

[css/text/bidi-inline-atoms]: ../tests/cases/css/text/bidi-inline-atoms/case.json

[css/text/breaking-products]: ../tests/cases/css/text/breaking-products/case.json

[css/text/dynamic-axes]: ../tests/cases/css/text/dynamic-axes/case.json

[css/text/dynamic-bidi-atoms]: ../tests/cases/css/text/dynamic-bidi-atoms/case.json

[css/text/dynamic-breaking]: ../tests/cases/css/text/dynamic-breaking/case.json

[css/text/dynamic-nested-decorations]: ../tests/cases/css/text/dynamic-nested-decorations/case.json

[css/text/font-features]: ../tests/cases/css/text/font-features/case.json

[css/text/nested-decorations]: ../tests/cases/css/text/nested-decorations/case.json

[css/text/relative-inheritance]: ../tests/cases/css/text/relative-inheritance/case.json

[css/transform/dynamic-order-clipping]: ../tests/cases/css/transform/dynamic-order-clipping/case.json

[css/transform/dynamic-perspective]: ../tests/cases/css/transform/dynamic-perspective/case.json

[css/transform/order-clipping]: ../tests/cases/css/transform/order-clipping/case.json

[css/transform/perspective-groups]: ../tests/cases/css/transform/perspective-groups/case.json

[css/transforms/dynamic-path-extents]: ../tests/cases/css/transforms/dynamic-path-extents/case.json

[css/transforms/path-extents]: ../tests/cases/css/transforms/path-extents/case.json

[css/values/dynamic-inherited-units]: ../tests/cases/css/values/dynamic-inherited-units/case.json

[css/values/inherited-units]: ../tests/cases/css/values/inherited-units/case.json
