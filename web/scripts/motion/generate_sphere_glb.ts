/** Build the deterministic sphere used by the I01 Metal/Raster acceptance test. */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { ROOT } from "./common.ts";

type SphereOptions = {
  segments?: number;
  rings?: number;
  withUvs?: boolean;
  generator?: string;
};

function pad(bytes: Uint8Array, fill: number): Uint8Array {
  const needed = (4 - bytes.length % 4) % 4;
  if (!needed) return bytes;
  const output = new Uint8Array(bytes.length + needed);
  output.set(bytes); output.fill(fill, bytes.length);
  return output;
}

function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
      .map(([key, part]) => `${JSON.stringify(key)}:${stableJson(part)}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

export function sphereGlb({
  segments = 32,
  rings = 16,
  withUvs = false,
  generator = "Valle I01 sphere",
}: SphereOptions = {}): Uint8Array {
  const positions: number[] = [];
  const uvs: number[] = [];
  const indices: number[] = [];
  for (let ring = 0; ring <= rings; ring++) {
    const latitude = Math.PI * ring / rings;
    for (let segment = 0; segment <= segments; segment++) {
      const longitude = 2 * Math.PI * segment / segments;
      positions.push(
        Math.sin(latitude) * Math.cos(longitude),
        Math.cos(latitude),
        Math.sin(latitude) * Math.sin(longitude),
      );
      if (withUvs) uvs.push(segment / segments, ring / rings);
    }
  }
  for (let ring = 0; ring < rings; ring++) {
    for (let segment = 0; segment < segments; segment++) {
      const a = ring * (segments + 1) + segment;
      const b = a + segments + 1;
      indices.push(a, a + 1, b, a + 1, b + 1, b);
    }
  }

  const floats = (values: number[]): Uint8Array => {
    const bytes = new Uint8Array(values.length * 4);
    const view = new DataView(bytes.buffer);
    values.forEach((value, index) => view.setFloat32(index * 4, value, true));
    return bytes;
  };
  const positionsBytes = floats(positions);
  const uvBytes = withUvs ? floats(uvs) : undefined;
  const indexBytes = new Uint8Array(indices.length * 2);
  const indexData = new DataView(indexBytes.buffer);
  indices.forEach((value, index) => indexData.setUint16(index * 2, value, true));
  const chunks = uvBytes
    ? [positionsBytes, positionsBytes, uvBytes, indexBytes]
    : [positionsBytes, positionsBytes, indexBytes];
  const offsets = [0];
  for (const chunk of chunks) offsets.push(offsets.at(-1)! + chunk.length);
  const joined = new Uint8Array(offsets.at(-1)!);
  chunks.forEach((chunk, index) => joined.set(chunk, offsets[index]!));
  const binary = pad(joined, 0);
  const indexView = withUvs ? 3 : 2;
  const document = {
    asset: { version: "2.0", generator },
    buffers: [{ byteLength: binary.length }],
    bufferViews: chunks.map((chunk, index) => ({
      buffer: 0,
      byteOffset: offsets[index],
      byteLength: chunk.length,
      target: index === indexView ? 34963 : 34962,
    })),
    accessors: [
      { bufferView: 0, componentType: 5126, count: positions.length / 3,
        type: "VEC3", min: [-1, -1, -1], max: [1, 1, 1] },
      { bufferView: 1, componentType: 5126, count: positions.length / 3, type: "VEC3" },
      ...(withUvs ? [{ bufferView: 2, componentType: 5126, count: uvs.length / 2, type: "VEC2" }] : []),
      { bufferView: indexView, componentType: 5123, count: indices.length, type: "SCALAR" },
    ],
    meshes: [{ primitives: [{ attributes: withUvs
      ? { POSITION: 0, NORMAL: 1, TEXCOORD_0: 2 }
      : { POSITION: 0, NORMAL: 1 }, indices: withUvs ? 3 : 2 }] }],
    nodes: [{ mesh: 0 }],
    scenes: [{ nodes: [0] }],
    scene: 0,
  };
  // Preserve the byte representation of the earlier I01 fixture, including its
  // floating-point bounds, so the regression test keeps the same model input.
  let documentJson = stableJson(document);
  if (!withUvs) {
    documentJson = documentJson.replace('"max":[1,1,1]', '"max":[1.0,1.0,1.0]')
      .replace('"min":[-1,-1,-1]', '"min":[-1.0,-1.0,-1.0]');
  }
  const json = pad(new TextEncoder().encode(documentJson), 0x20);
  const output = new Uint8Array(12 + 8 + json.length + 8 + binary.length);
  output.set(new TextEncoder().encode("glTF"));
  const header = new DataView(output.buffer);
  header.setUint32(4, 2, true);
  header.setUint32(8, output.length, true);
  header.setUint32(12, json.length, true);
  header.setUint32(16, 0x4e4f534a, true);
  output.set(json, 20);
  const binaryHeader = 20 + json.length;
  header.setUint32(binaryHeader, binary.length, true);
  header.setUint32(binaryHeader + 4, 0x004e4942, true);
  output.set(binary, binaryHeader + 8);
  return output;
}

if (import.meta.main) {
  const output = resolve(Bun.argv[2] ?? join(ROOT, "target/motion-acceptance/sphere.glb"));
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, sphereGlb());
  console.log(output);
}
