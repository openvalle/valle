import type { Scene3dGpuRequest } from "../../executor/webgl/scene3d.ts";

/** Closed eligibility for the reference-projected procedural GPU rasterizer. */
export function supportsScene3dGpu(r: Scene3dGpuRequest): boolean {
  return (
    !r.scene.pbr?.environment &&
    r.scene.meshes.every(
      (m) =>
        m.geometry &&
        !m.modelControl &&
        !m.nodeIds?.length &&
        m.animationClip == null &&
        !m.materialOverrides?.length &&
        !Object.keys(m.material.textures ?? {}).length &&
        (!m.material.alphaMode || m.material.alphaMode === "opaque"),
    ) &&
    r.frame.lights.filter((l) => l.type === "directional").length <= 2 &&
    r.frame.lights.every((l) => ["directional", "ambient", "hemisphere"].includes(l.type)) &&
    r.frame.meshes.every(
      (m) =>
        !m.nodes?.length &&
        !m.materialOverrides?.length &&
        (m.material.color?.[3] ?? 1) === 1 &&
        m.transform.scale.every((v) => v > 0),
    )
  );
}
