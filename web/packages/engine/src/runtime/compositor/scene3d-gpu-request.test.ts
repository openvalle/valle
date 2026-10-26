import { expect, test } from "bun:test";
import type { Scene3dGpuRequest } from "../../executor/webgl/scene3d.ts";
import { scene3dHasExternalResources, supportsScene3dGpu } from "./scene3d-gpu-request.ts";

test("procedural material overrides wait for the authoritative resource registry", () => {
  const request: Scene3dGpuRequest = {
    width: 3840,
    height: 2160,
    scene: { meshes: [{ geometry: {}, material: { kind: "pbr" } }] },
    frame: {
      meshes: [{ transform: { scale: [1, 1, 1] }, material: {} }],
      lights: [],
      exposure: 1,
      camera: { far: 100 },
    },
  };
  expect(supportsScene3dGpu(request)).toBe(true);
  expect(scene3dHasExternalResources(request)).toBe(false);
  const mesh = request.scene.meshes[0]!;
  const override = { id: 0, material: { textures: { baseColor: { control: "map" } } } };
  expect(
    scene3dHasExternalResources({
      ...request,
      scene: { meshes: [{ ...mesh, materialOverrides: [override] }] },
    }),
  ).toBe(true);
  expect(
    scene3dHasExternalResources({
      ...request,
      scene: { meshes: [{ ...mesh, modelControl: "model" }] },
    }),
  ).toBe(true);
  expect(
    scene3dHasExternalResources({
      ...request,
      scene: { meshes: [mesh], pbr: { environment: {} } },
    }),
  ).toBe(true);
  expect(supportsScene3dGpu({ ...request, width: 4097 })).toBe(false);
});
