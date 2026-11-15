# Curved glyph outlines

SVG 1.1 `textPath method="stretch"` maps the outline's inline coordinate to
baseline arclength and its block coordinate to distance along the local normal.
The upper row uses rigid `align`; the lower row must visibly bend the same letters.

The reviewed Chromium, Firefox and WebKit probes all rendered `stretch` like
`align`. None is used as the correctness reference here. The reviewed Valle PNGs
protect the appearance, while `svg/text/stretch.rs` independently checks exact
straight-line equivalence, endpoint tangent extension and constant normal distance
on a quadratic baseline. The ordinary textPath browser cases continue to cover
rigid glyph placement.
