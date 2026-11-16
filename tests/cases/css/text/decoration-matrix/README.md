# decoration-matrix

Full product: 5 decoration styles × 3 writing modes × 2 opacity values = 30.

An explicit under/left underline position avoids the font-dependent auto position.
The parent owns the underline; the child changes fill and stroke width.
Opacity applies after stroke, shadow and decoration. This compares the final
picture, not only parser admission, for horizontal and both vertical directions.

Fonts are pinned. Browser images are compared and reviewed before selecting a
baseline. Rasterization allowances must not hide missing lines or shifted boxes.

## Reviewed differences

The browser comparison allows glyph rasterization, roughly one-pixel underline
placement and decoration phase differences. It is deliberately paired with a
reviewed Valle PNG (maximum channel delta 1): changing or removing a decoration
must still fail. The GPU mutation test removes the underline to verify this.
Mixed-size central-baseline alignment has its own `mixed-size-matrix` so it is
not hidden in this decoration allowance.
