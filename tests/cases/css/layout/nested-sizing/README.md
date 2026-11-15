# css/layout/nested-sizing

Full product: 2 outer layouts × 2 inner layouts × 2 sizing modes = 8.

The inner max-content width is 50 + 10 + 70 + 16 = 146. A definite outer
content width of 208 makes 50% equal 104, clamped by the inner 146px minimum.
An intrinsic outer width adds 12px of its own border and padding to 146.
No expected value is taken from a browser or Valle measurement.

The TSX and ordinary browser CSS describe the same inputs. Box expectations are
calculated from the rules above, not copied from either renderer. All tuples are
retained in the scene; no test input is randomly regenerated.
