# SVG patterns

Inline and file cases exercise user-space and bounding-box tiles, transforms, viewBox
fitting, href reuse, fill opacity and patterned strokes. The inline tile rotation also
runs on backward WebGPU seeks.

SVG 2 section 14.1.1 specifies a shadow tree for inherited pattern contents. Current
Chrome, Firefox and WebKit keep the source parent's inherited paint instead. The image
case uses an explicit child fill, common to all three; the unit test
`patterns_resolve_coordinates_references_and_cycles` separately asserts the specified
host inheritance, along with degenerate sizes and cyclic references.
