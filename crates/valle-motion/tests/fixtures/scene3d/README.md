# Scene3D binding fixtures

`triangle.glb` is a self-contained GLB 2.0 triangle with positions (-0.9,-0.7,0),
(0.9,-0.7,0), (0,0.9,0), normals (0,0,1), UVs (0,1), (1,1), (0.5,0),
and U16 indices 0,1,2. `narrow.glb` halves only the X positions. Both are authored
test data, without third-party assets, materials or external paths.

The Motion source rotates the triangle from the target frame's local time.
`pbr.glb` uses the same triangle with five authored 1×1 PNG material maps (base
color, metallic/roughness, emissive, occlusion and normal).

Native and Web tests construct their frozen packages at run time. Environment
panoramas are generated in test code; no environment image or serialized package
snapshot is checked in.
