/** Closed eligibility for the reference-projected procedural GPU rasterizer. */
export function supportsScene3dGpu(r: Record<string, any>): boolean {
  return (
    !r.scene.pbr?.environment &&
    r.scene.meshes.every(
      (m: Record<string, any>) =>
        m.geometry &&
        !m.modelControl &&
        !m.nodeIds?.length &&
        m.animationClip == null &&
        !m.materialOverrides?.length &&
        !Object.keys(m.material.textures ?? {}).length &&
        (!m.material.alphaMode || m.material.alphaMode === "opaque"),
    ) &&
    r.frame.lights.filter((l: Record<string, any>) => l.type === "directional")
      .length <= 2 &&
    r.frame.lights.every((l: Record<string, any>) =>
      ["directional", "ambient", "hemisphere"].includes(l.type),
    ) &&
    r.frame.meshes.every(
      (m: Record<string, any>) =>
        !m.nodes?.length &&
        !m.materialOverrides?.length &&
        (m.material.color?.[3] ?? 1) === 1 &&
        m.transform.scale.every((v: number) => v > 0),
    )
  );
}
