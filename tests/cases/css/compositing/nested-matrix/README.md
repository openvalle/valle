# nested-matrix

32 combinations: isolation × parent opacity × overflow × empty/content × inner blend. Nested backdrop regions extend outside the parent so grouping, clipping and empty layers remain observable.

This is the full product of the factors in `CASES`, not random sampling.
Parent rectangles have independent numerical expectations. The three browser
captures and native/difference pictures are reviewed before accepting pixels.

The initial render incorrectly flattened nested normal groups by multiplying
their opacities. That removed the empty backdrop root and produced gray filtered
regions. `paint::group` now retains boundaries when the child reads a backdrop.
After the fix, this entire fixture matches Chrome exactly; existing normal-group
flattening remains covered by the unit test.
