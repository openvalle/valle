# SVG backgrounds and round positioning

The last control requests 40×20px tiles in a 100×80px positioning area.
CSS Backgrounds §2.9 resizes the width to `100 / round(100 / 40) = 100/3px`.
Section 2.6 then positions that resized image: centered X is
`(100 - 100/3) / 2 = 100/3px`, equivalent to phase zero for this repeating tile.

The retained Firefox capture uses this position. Chrome and WebKit use the old
40px width for positioning, shifting the repeated pattern by 10/3px. The earlier
Chrome selection matched that behavior and was invalidated by the common tiling
fix. All original browser captures are retained; this is a reviewed baseline
selection change, not a recapture or an unexplained tolerance increase.

The actual/Firefox difference image contains SVG tile seams and rounded edges,
not a shifted pattern. A strict Valle baseline and a mutation removing round
resizing protect geometry despite the browser antialiasing allowance. The shared
tiling function also has direct numerical tests for resizing and positioning.

Specification: https://www.w3.org/TR/css-backgrounds-3/#the-background-size
and https://www.w3.org/TR/css-backgrounds-3/#the-background-position.
