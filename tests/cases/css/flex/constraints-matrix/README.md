# css/flex/constraints-matrix

Full product: 3 widths × 2 gaps × 2 box models × 2 directions = 24.

The container content width is width − 12. Before clamping, each item receives
(content − gap)/2. The first border-box width clamps to [80,100], or [88,108]
for content-box because its padding and borders add 8. The second receives the
remainder. Row-reverse exchanges physical placement, not the constraints.

The TSX and ordinary browser CSS describe the same inputs. Box expectations are
calculated from the rules above, not copied from either renderer. All tuples are
retained in the scene; no test input is randomly regenerated.
