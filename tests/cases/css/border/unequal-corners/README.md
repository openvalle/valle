# unequal-corners

32 combinations cross all eight visible border styles, equal/unequal widths and square/asymmetric corners. Different edge colors expose joins and clipping.

8 styles × 2 width lists × 2 radius lists. Compare geometry separately from browser-specific dash phase and edge antialiasing. Solid and double controls isolate corner coverage.

The listed inputs are permanent. Browser HTML uses ordinary CSS and does not
consume Valle layout results. Review the three browser images and native
difference images before accepting the baseline.

Reviewed all 32 cells and difference maps. Solid interiors agree. Differences lie on curved joins, groove/ridge seams and dotted/dashed placement; the latter is not prescribed as an exact pixel pattern.
The strict Valle baseline prevents the browser rasterization allowance from
accepting removal of a border or shadow.
