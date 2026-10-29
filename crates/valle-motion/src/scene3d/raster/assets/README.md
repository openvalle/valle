# PBR algorithm data

`dfg.rg16f` is the generic 16×16 integrated BRDF table from Three.js r186 `src/renderers/shaders/DFGLUTData.js`, stored as little-endian RG16F and decoded at compile time.

The BRDF table and adapted ACES/multiscattering calculations retain their MIT attribution in `THREE-LICENSE.txt`. Environment lighting comes from project resources and is prefiltered by the shared Rust environment module.

Source: https://github.com/mrdoob/three.js/tree/r186

`lighting.rgba16f` is a 128×80 little-endian RGBA16F lookup texture shared by native
GPU and WebGL. The original DFG values occupy the RG channels of its upper-left
16×16 region. Rows 16–79 contain the two 64×64 LTC tables side by side: inverse
GGX transforms on the left, Fresnel amplitudes on the right. Padding is zero.
The CPU decodes the same half values before interpolation.

LTC data source (MIT): Three.js r186
`examples/jsm/lights/RectAreaLightTexturesLib.js`, `LTC_MAT_1` and `LTC_MAT_2`.
Source SHA-256: `6dd4043bb052594357a5eeff13dc0519f0a80b100b97ec80c12808f887897bc1`.
Packing uses IEEE binary16 round-to-nearest-even; packed SHA-256:
`c00d2c79efc7681916e56c51ddeb080c7635ea9822c6ac3ab63435dcf14e0b41`.
The rectangle integration follows Heitz, Dupuy, Hill and Neubelt,
“Real-Time Polygonal-Light Shading with Linearly Transformed Cosines”, with the
rational edge approximation from the attributed Three.js code. Valle explicitly
clips the transformed polygon at the horizon before integration.
