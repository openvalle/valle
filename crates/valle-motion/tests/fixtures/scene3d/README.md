# Scene3D binding fixtures

`triangle.glb` is a self-contained GLB 2.0 triangle with positions (-0.9,-0.7,0),
(0.9,-0.7,0), (0,0.9,0), normals (0,0,1), UVs (0,1), (1,1), (0.5,0),
and U16 indices 0,1,2. `narrow.glb` halves only the X positions. Both are authored
test data, without third-party assets, materials or external paths.

`sphere.glb` is a self-contained unit sphere with 32 longitudinal segments and
16 latitude rings, containing 561 vertices, vertex normals and U16 indices for
1,024 triangles. It has no UVs, materials or external resources. The I01
1080p Raster/Metal acceptance test reads this fixed model directly. It was
generated with the default options of
`web/scripts/motion/generate_sphere_glb.ts` and is authored test data.

The Motion source rotates the triangle from the target frame's local time.
`pbr.glb` uses the same triangle with five authored 1×1 PNG material maps (base
color, metallic/roughness, emissive, occlusion and normal).

Native and Web tests construct their frozen packages at run time. Environment
panoramas are generated in test code; no environment image or serialized package
snapshot is checked in.

`box-animated.glb` is the [Khronos glTF Sample Assets Box Animated model](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/BoxAnimated),
copyright 2017 Cesium, licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
It contains one animation with rotation and translation channels of different lengths.

`animated-morph-cube.glb` is the [Khronos Animated Morph Cube](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/AnimatedMorphCube),
released to the public domain under [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).
It contains two POSITION, NORMAL and TANGENT morph targets and one animated weights channel.

`rigged-simple.glb` is the [Khronos Rigged Simple model](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/RiggedSimple),
copyright 2017 Cesium, licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
It contains a two-joint skin, inverse bind matrices and animated joint TRS channels.
