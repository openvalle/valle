# transformed-bounds

18 combinations: 3 filters × 3 transforms × ancestor overflow. Blur and shadow bounds extend beyond the source and must be clipped after the child transform by the rounded ancestor.

This is the full product of the factors in `CASES`, not random sampling.
Parent rectangles have independent numerical expectations. The three browser
captures and native/difference pictures are reviewed before accepting pixels.
