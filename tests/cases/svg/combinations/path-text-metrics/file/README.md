# combinations/path-text-metrics

18 combinations cross text-anchor, dominant-baseline and lengthAdjust on a curved textPath with a fixed font and explicit textLength.

Inputs, in row order:

```json
[
  {
    "anchor": "start",
    "baseline": "alphabetic",
    "length": "spacing"
  },
  {
    "anchor": "start",
    "baseline": "alphabetic",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "start",
    "baseline": "middle",
    "length": "spacing"
  },
  {
    "anchor": "start",
    "baseline": "middle",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "start",
    "baseline": "hanging",
    "length": "spacing"
  },
  {
    "anchor": "start",
    "baseline": "hanging",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "middle",
    "baseline": "alphabetic",
    "length": "spacing"
  },
  {
    "anchor": "middle",
    "baseline": "alphabetic",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "middle",
    "baseline": "middle",
    "length": "spacing"
  },
  {
    "anchor": "middle",
    "baseline": "middle",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "middle",
    "baseline": "hanging",
    "length": "spacing"
  },
  {
    "anchor": "middle",
    "baseline": "hanging",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "end",
    "baseline": "alphabetic",
    "length": "spacing"
  },
  {
    "anchor": "end",
    "baseline": "alphabetic",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "end",
    "baseline": "middle",
    "length": "spacing"
  },
  {
    "anchor": "end",
    "baseline": "middle",
    "length": "spacingAndGlyphs"
  },
  {
    "anchor": "end",
    "baseline": "hanging",
    "length": "spacing"
  },
  {
    "anchor": "end",
    "baseline": "hanging",
    "length": "spacingAndGlyphs"
  }
]
```

Inline and file fixtures use equivalent SVG. The file input exercises XML decoding and reference resolution; the inline input exercises JSX lowering. Both keep independently captured browser images.
