import type { Scene3dGpuRequest } from "../../executor/webgl/scene3d.ts";

/** Backend size admission; semantic/material validation remains in Rust. */
export function supportsScene3dGpu(r: Scene3dGpuRequest): boolean {
  return (
    Number.isSafeInteger(r.width) &&
    Number.isSafeInteger(r.height) &&
    r.width > 0 &&
    r.height > 0 &&
    r.width <= 4096 &&
    r.height <= 4096 &&
    r.width * r.height <= 8_388_608
  );
}

/** Stateless geometry workers have no external resource registry. */
export function scene3dHasExternalResources(r: Scene3dGpuRequest): boolean {
  return (
    !!r.scene.pbr?.environment ||
    r.scene.meshes.some(
      (m) =>
        !!m.modelControl ||
        !!m.materialOverrides?.length ||
        Object.keys(m.material.textures ?? {}).length > 0,
    )
  );
}
