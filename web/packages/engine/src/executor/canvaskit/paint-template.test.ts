import { expect, test } from "bun:test";
import { DRAW_PROGRAM_ABI, drawProgramPaint, type TaggedWire } from "./draw-program.ts";
import {
  decodePaintTemplate,
  bindPaintParameters,
  paintDigest,
  subtreeParameterKey,
} from "./paint-template.ts";
import { encodeTestPaths } from "./path-arena.test-helper.ts";
import { staticSubtreeIdentities } from "./static-subtree-identity.ts";

function fixture(nodes: TaggedWire[] = []): Uint8Array {
  const json = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
  const sections = [
    json(nodes.length === 0 ? [] : [nodes.length - 1]),
    json(nodes),
    new Uint8Array(),
    new Uint8Array(),
    new Uint8Array(),
    encodeTestPaths(
      nodes.length === 0
        ? []
        : [
            {
              verbs: ["moveTo", "lineTo", "lineTo", "close"],
              points: [
                [0, 0],
                [64, 0],
                [64, 64],
              ],
            },
          ],
    ),
    json([
      { kind: "solid", value: { red: 0, green: 0, blue: 0, alpha: 0 } },
      { kind: "solid", value: { red: 0.5, green: 0, blue: 0, alpha: 1 } },
    ]),
    json({ x: 0, y: 0, width: 64, height: 64 }),
    json({}),
  ];
  const offset =
    DRAW_PROGRAM_ABI.headerBytes + sections.length * DRAW_PROGRAM_ABI.sectionEntryBytes;
  const draw = new Uint8Array(offset + sections.reduce((sum, bytes) => sum + bytes.length, 0));
  const view = new DataView(draw.buffer);
  draw.set(new TextEncoder().encode(DRAW_PROGRAM_ABI.magic));
  view.setUint32(8, DRAW_PROGRAM_ABI.formatVersion, true);
  view.setUint32(12, DRAW_PROGRAM_ABI.endiannessMarker, true);
  view.setUint16(16, sections.length, true);
  view.setBigUint64(20, BigInt(draw.length), true);
  let cursor = offset;
  for (const [index, bytes] of sections.entries()) {
    const at = DRAW_PROGRAM_ABI.headerBytes + index * DRAW_PROGRAM_ABI.sectionEntryBytes;
    view.setUint16(at, [1, 2, 7, 8, 9, 3, 4, 5, 6][index]!, true);
    view.setUint32(
      at + 4,
      [nodes.length === 0 ? 0 : 1, nodes.length, 0, 0, 0, nodes.length === 0 ? 0 : 1, 2, 1, 1][
        index
      ]!,
      true,
    );
    view.setBigUint64(at + 8, BigInt(cursor), true);
    view.setBigUint64(at + 16, BigInt(bytes.length), true);
    draw.set(bytes, cursor);
    cursor += bytes.length;
  }
  const bytes = new Uint8Array(24 + draw.length);
  bytes.set(new TextEncoder().encode("VLPAINT1"));
  new DataView(bytes.buffer).setUint32(8, 1, true);
  new DataView(bytes.buffer).setUint32(12, 1, true);
  bytes.set(draw, 24);
  return bytes;
}

async function frameDigest(template: Uint8Array, parameters: Uint8Array) {
  const domain = new TextEncoder().encode("valle.paint-frame@1\0");
  const bytes = new Uint8Array(domain.length + 32 + parameters.length);
  bytes.set(domain);
  bytes.set(
    new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(template))),
    domain.length,
  );
  bytes.set(parameters, domain.length + 32);
  return paintDigest(bytes);
}

test("static duplicates share cache identity without merging dynamic consumers", async () => {
  const nodes: TaggedWire[] = [
    { kind: "path", value: { path: 0, fill: 1 } },
    { kind: "path", value: { path: 0, fill: 1 } },
    { kind: "path", value: { path: 0, fill: 0 } },
    { kind: "path", value: { path: 0, fill: 0 } },
    {
      kind: "group",
      value: { children: [0], mask: { source: 1, mode: "alpha" }, internalBlend: "normal" },
    },
    {
      kind: "group",
      value: { children: [1], mask: { source: 0, mode: "alpha" }, internalBlend: "normal" },
    },
    {
      kind: "group",
      value: { children: [1], mask: { source: 0, mode: "luminance" }, internalBlend: "normal" },
    },
  ];
  const bytes = fixture(nodes);
  const parameters = new Uint8Array(16);
  new DataView(parameters.buffer).setFloat32(12, 1, true);
  const capture = async () => {
    const template = await decodePaintTemplate(bytes, await paintDigest(bytes));
    const draw = await bindPaintParameters(
      template,
      parameters,
      await frameDigest(bytes, parameters),
    );
    return nodes.map((_, id) => subtreeParameterKey(draw, [id]));
  };
  const keys = await capture();
  expect(keys[0]).not.toBeNull();
  expect(keys[0]).toBe(keys[1]);
  expect(keys[2]).not.toBe(keys[3]);
  expect(keys[4]).toBe(keys[5]);
  expect(keys[4]).not.toBe(keys[6]);
  expect(await capture()).toEqual(keys);
});

test("static structural identities preserve reference contents, order and exact numbers", async () => {
  const bytes = fixture([{ kind: "path", value: { path: 0, fill: 1 } }]);
  const draw = structuredClone((await decodePaintTemplate(bytes, await paintDigest(bytes))).draw);
  draw.paths.push(structuredClone(draw.paths[0]!));
  draw.paints.push(structuredClone(draw.paints[1]!));
  const node = (path: number, fill: number): TaggedWire => ({
    kind: "path",
    value: { path, fill },
  });
  draw.nodes = [node(0, 1), node(1, 2)];
  const ids = () => staticSubtreeIdentities(draw, () => []);
  expect(ids()[0]).toBe(ids()[1]);
  draw.paths[1]!.points[0]![0] = -0;
  expect(ids()[0]).not.toBe(ids()[1]);
  draw.paths[1]!.points[0]![0] = 0;
  (draw.paints[2]!.value as Record<string, number>).red = 0.5000000000000001;
  expect(ids()[0]).not.toBe(ids()[1]);
  draw.nodes.push(
    { kind: "group", value: { children: [0, 1] } },
    { kind: "group", value: { children: [1, 0] } },
    { kind: "instanceBatch", value: {} },
  );
  expect(ids()[2]).not.toBe(ids()[3]);
  expect(ids()[4]).toBeNull();
  expect(staticSubtreeIdentities(draw, () => null)).toEqual([null, null, null, null, null]);
});

test("subtree keys read masks and strokes, ignore unrelated paint, and preserve admitted bits", async () => {
  const nodes: TaggedWire[] = [
    { kind: "path", value: { path: 0, fill: 0 } },
    { kind: "path", value: { path: 0, fill: 1 } },
    {
      kind: "group",
      value: { children: [1], mask: { source: 0, mode: "alpha" }, internalBlend: "normal" },
    },
    { kind: "path", value: { path: 0, fill: 1, stroke: { paint: 0, width: 1 } } },
    { kind: "image", value: {} },
    { kind: "group", value: { children: [1], internalBlend: "multiply" } },
    { kind: "group", value: { children: [2, 3, 4, 5], internalBlend: "normal" } },
  ];
  const bytes = fixture(nodes);
  const template = await decodePaintTemplate(bytes, await paintDigest(bytes));
  const keys = [];
  for (const red of [0, -0, 0.25, 0.25000003, 0]) {
    const parameters = new Uint8Array(16);
    const view = new DataView(parameters.buffer);
    view.setFloat32(0, red, true);
    view.setFloat32(12, 1, true);
    const draw = await bindPaintParameters(
      template,
      parameters,
      await frameDigest(bytes, parameters),
    );
    const row = nodes.map((_, index) => subtreeParameterKey(draw, [index]));
    expect(row[4]).toBeNull();
    expect(row[5]).toBeNull();
    expect(row[6]).toBeNull();
    expect(subtreeParameterKey(draw, [9999])).toBeNull();
    expect(subtreeParameterKey(draw, [0, 1])).not.toBe(subtreeParameterKey(draw, [1, 0]));
    // Mutating the caller's transport buffer cannot silently change a retained frame's key.
    parameters.fill(0);
    expect(subtreeParameterKey(draw, [0])).toBe(row[0]);
    keys.push(row);
  }
  for (const row of keys.slice(1, 4)) {
    expect(row[1]).toBe(keys[0]![1]);
    for (const root of [0, 2, 3]) expect(row[root]).not.toBe(keys[0]![root]);
  }
  expect(keys[0]).toEqual(keys[4]);
});

test("paint frames preserve f32 bits, share geometry, and leave earlier frames immutable", async () => {
  const bytes = fixture();
  const template = await decodePaintTemplate(bytes, await paintDigest(bytes));
  const frames = [];
  for (const red of [0.25, 4, -0, -0.25, 0.25]) {
    const params = new Uint8Array(16);
    const view = new DataView(params.buffer);
    view.setFloat32(0, red, true);
    view.setFloat32(12, 1, true);
    const frame = await bindPaintParameters(template, params, await frameDigest(bytes, params));
    expect(frame.nodes).toBe(template.draw.nodes);
    expect(frame.paths).toBe(template.draw.paths);
    expect(frame.paints).toBe(template.draw.paints);
    expect(drawProgramPaint(frame, 1)).toBe(template.draw.paints[1]);
    expect(Object.is((drawProgramPaint(frame, 0)!.value as { red: number }).red, red)).toBe(true);
    frames.push(frame);
  }
  expect((drawProgramPaint(frames[0]!, 0)!.value as { red: number }).red).toBe(0.25);
  expect((template.draw.paints[0]!.value as { red: number }).red).toBe(0);
  const cold = await decodePaintTemplate(bytes, await paintDigest(bytes));
  expect(cold.draw).toEqual(template.draw);
});

test("malformed templates, nonfinite colors, length and identity mismatches fail admission", async () => {
  const bytes = fixture();
  const template = await decodePaintTemplate(bytes, await paintDigest(bytes));
  for (const [offset, value] of [
    [8, 0xffffffff],
    [12, 0xffffffff],
    [16, 1],
    [20, 1],
  ]) {
    const bad = bytes.slice();
    new DataView(bad.buffer).setUint32(offset!, value!, true);
    await expect(decodePaintTemplate(bad, await paintDigest(bad))).rejects.toThrow();
  }
  for (const length of [0, 7, 15, 23, bytes.length - 1]) {
    const bad = bytes.subarray(0, length);
    await expect(decodePaintTemplate(bad, await paintDigest(bad))).rejects.toThrow();
  }
  await expect(decodePaintTemplate(bytes, "sha256:wrong")).rejects.toThrow();
  for (const params of [new Uint8Array(32), new Uint8Array(16).fill(255)]) {
    await expect(
      bindPaintParameters(template, params, await frameDigest(bytes, params)),
    ).rejects.toThrow();
  }
  const params = new Uint8Array(16);
  new DataView(params.buffer).setFloat32(0, 1, true); // alpha zero with nonzero RGB
  await expect(
    bindPaintParameters(template, params, await frameDigest(bytes, params)),
  ).rejects.toThrow();
  for (const alpha of [-0.1, 1.1]) {
    params.fill(0);
    new DataView(params.buffer).setFloat32(12, alpha, true);
    await expect(
      bindPaintParameters(template, params, await frameDigest(bytes, params)),
    ).rejects.toThrow();
  }
  params.fill(0);
  await expect(bindPaintParameters(template, params, "sha256:wrong")).rejects.toThrow();
});
