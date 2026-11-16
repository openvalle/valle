# outline-layers

12 combinations cross clipping, rotation and multiple inner/outer shadows with an offset outline. Rounded clipping applies to the complete transformed child.

2 overflow modes × 2 transforms × 3 shadow lists. Sharp shadows are controls for blurred coverage; outlines remain outside the border box.

The listed inputs are permanent. Browser HTML uses ordinary CSS and does not
consume Valle layout results. Review the three browser images and native
difference images before accepting the baseline.

All 12 clipping/transform/shadow combinations agree in extent and ordering. Differences follow dashed outline phase, curve coverage and blurred shadow falloff.
The strict Valle baseline prevents the browser rasterization allowance from
accepting removal of a border or shadow.
