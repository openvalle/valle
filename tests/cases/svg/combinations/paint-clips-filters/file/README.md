# combinations/paint-clips-filters

16 combinations cross gradient coordinate systems, group opacity, clipping and a two-stage filter over patterned content.

Inputs, in row order:

```json
[
  {
    "units": "objectBoundingBox",
    "filter": false,
    "clip": false,
    "opacity": 0.5
  },
  {
    "units": "objectBoundingBox",
    "filter": false,
    "clip": false,
    "opacity": 1
  },
  {
    "units": "objectBoundingBox",
    "filter": false,
    "clip": true,
    "opacity": 0.5
  },
  {
    "units": "objectBoundingBox",
    "filter": false,
    "clip": true,
    "opacity": 1
  },
  {
    "units": "objectBoundingBox",
    "filter": true,
    "clip": false,
    "opacity": 0.5
  },
  {
    "units": "objectBoundingBox",
    "filter": true,
    "clip": false,
    "opacity": 1
  },
  {
    "units": "objectBoundingBox",
    "filter": true,
    "clip": true,
    "opacity": 0.5
  },
  {
    "units": "objectBoundingBox",
    "filter": true,
    "clip": true,
    "opacity": 1
  },
  {
    "units": "userSpaceOnUse",
    "filter": false,
    "clip": false,
    "opacity": 0.5
  },
  {
    "units": "userSpaceOnUse",
    "filter": false,
    "clip": false,
    "opacity": 1
  },
  {
    "units": "userSpaceOnUse",
    "filter": false,
    "clip": true,
    "opacity": 0.5
  },
  {
    "units": "userSpaceOnUse",
    "filter": false,
    "clip": true,
    "opacity": 1
  },
  {
    "units": "userSpaceOnUse",
    "filter": true,
    "clip": false,
    "opacity": 0.5
  },
  {
    "units": "userSpaceOnUse",
    "filter": true,
    "clip": false,
    "opacity": 1
  },
  {
    "units": "userSpaceOnUse",
    "filter": true,
    "clip": true,
    "opacity": 0.5
  },
  {
    "units": "userSpaceOnUse",
    "filter": true,
    "clip": true,
    "opacity": 1
  }
]
```

Inline and file fixtures use equivalent SVG. The file input exercises XML decoding and reference resolution; the inline input exercises JSX lowering. Both keep independently captured browser images.
