# truncation-matrix

Products: 2 widths × 2 text-overflow values × 2 directions = 8, plus
2 font sizes × 2 clamp counts = 4.

Single-line clipping leaves the authored paragraph box unchanged. The clamp
controls use automatic height: one 32px line must occupy 32px, two must occupy
64px. A fixed height would leave later lines visible even after the marker and
would not test the intended clamp behavior.

DOM range measurements retain source text, so they are not treated as the
visible ellipsis/clamp contents. Box geometry, reviewed images and the RTL
truncation unit test provide independent checks. The original RTL implementation
only looked for overflow at the right edge; it now trims the visual left edge.
