# css/flex/wrap-thresholds

Full product: 3 threshold offsets × 2 gaps × 3 wrap modes × 2 directions = 36.

Two 60px items need 120 + gap content pixels. With 2px less, nowrap shrinks
each to 59px; wrap modes create two lines. At the threshold and 2px above,
items stay 60px. Each line is 20px tall; wrap-reverse starts at the content
bottom and row-reverse starts at the content right. Free space is not grown.

The TSX and ordinary browser CSS describe the same inputs. Box expectations are
calculated from the rules above, not copied from either renderer. All tuples are
retained in the scene; no test input is randomly regenerated.
