# PBR algorithm data

`dfg.rg16f` is the generic 16×16 integrated BRDF table from Three.js r186 `src/renderers/shaders/DFGLUTData.js`, stored as little-endian RG16F and decoded at compile time.

The BRDF table and adapted ACES/multiscattering calculations retain their MIT attribution in `THREE-LICENSE.txt`. Environment lighting comes from project resources and is prefiltered by the shared Rust environment module.

Source: https://github.com/mrdoob/three.js/tree/r186
