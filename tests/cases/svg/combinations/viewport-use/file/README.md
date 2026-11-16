# combinations/viewport-use

Eight symbol/use controls cross viewBox origin, preserveAspectRatio and an explicit clipPath with oversized child geometry.

Inputs, in row order:

```json
[
  {
    "aspect": "xMidYMid meet",
    "origin": 0,
    "clip": "visible"
  },
  {
    "aspect": "xMidYMid meet",
    "origin": 0,
    "clip": "hidden"
  },
  {
    "aspect": "xMidYMid meet",
    "origin": 10,
    "clip": "visible"
  },
  {
    "aspect": "xMidYMid meet",
    "origin": 10,
    "clip": "hidden"
  },
  {
    "aspect": "xMaxYMin slice",
    "origin": 0,
    "clip": "visible"
  },
  {
    "aspect": "xMaxYMin slice",
    "origin": 0,
    "clip": "hidden"
  },
  {
    "aspect": "xMaxYMin slice",
    "origin": 10,
    "clip": "visible"
  },
  {
    "aspect": "xMaxYMin slice",
    "origin": 10,
    "clip": "hidden"
  }
]
```

Inline and file fixtures use equivalent SVG. The file input exercises XML decoding and reference resolution; the inline input exercises JSX lowering. Both keep independently captured browser images.
