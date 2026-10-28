import { PAINT_TEMPLATE_ABI, DRAW_PROGRAM_ABI } from "../../generated/packed-abi.ts";
import {
  decodeDrawProgram,
  PackedDrawProgramError,
  type DrawProgramWire,
  type TaggedWire,
} from "./draw-program.ts";
import type { PathArenaCache } from "./path-arena-cache.ts";
import { staticSubtreeIdentities } from "./static-subtree-identity.ts";

/** VLPAINT1 is the canonical Rust PaintTemplate encoding; parameter lanes are exact LE f32. */
export interface PaintTemplateWire {
  readonly draw: DrawProgramWire;
  readonly digest: Uint8Array;
  readonly slots: number;
  readonly bindings: ReadonlyArray<readonly [number, number]>;
}

const parameterFrames = new WeakMap<
  DrawProgramWire,
  { template: PaintTemplateWire; bytes: Uint8Array }
>();
const dependencyIndices = new WeakMap<
  PaintTemplateWire,
  {
    nodes: Map<number, readonly number[] | null>;
    paints: Map<number, number>;
    staticIdentities?: Array<number | null>;
  }
>();

/** Exact paint reads of destination-independent, resource-free subtrees. Geometry and static
 * colors are already identified by the admitted template digest. Unknown node classes fail
 * closed; image/font/shader/scene resources need resolved host identities before admission. */
export function subtreeParameterKey(
  draw: DrawProgramWire,
  roots: readonly number[],
): string | null {
  const frame = parameterFrames.get(draw);
  if (!frame) return null;
  let index = dependencyIndices.get(frame.template);
  if (!index) {
    index = { nodes: new Map(), paints: new Map(frame.template.bindings) };
    dependencyIndices.set(frame.template, index);
  }
  const { nodes: memo, paints } = index;
  const visit = (id: number, depth = 0): readonly number[] | null => {
    if (memo.has(id)) return memo.get(id)!;
    if (depth > 128 || !Number.isSafeInteger(id) || id < 0) return null;
    const node = frame.template.draw.nodes[id];
    if (!node) return null;
    const value = node.value as Record<string, any>;
    const slots = new Set<number>();
    const paint = (id: number | null | undefined) => {
      if (id == null) return;
      const slot = paints.get(id);
      if (slot !== undefined) slots.add(slot);
    };
    // Install a sentinel before descent: forged cyclic node graphs cannot recurse forever.
    memo.set(id, null);
    switch (node.kind) {
      case "path":
        paint(value.fill);
        paint(value.stroke?.paint);
        break;
      case "group": {
        if (
          value.backdrop != null ||
          value.internalBlend !== "normal" ||
          value.shader != null ||
          value.transition?.kernel?.kind === "shader"
        )
          return null;
        const children = [...value.children];
        if (value.mask != null) children.push(value.mask.source);
        for (const child of children) {
          const dependencies = visit(child, depth + 1);
          if (dependencies === null) return null;
          for (const slot of dependencies) slots.add(slot);
        }
        break;
      }
      case "instanceBatch":
        if (value.shape?.kind === "image") return null;
        break;
      case "shadow":
        break;
      default:
        return null;
    }
    const result = [...slots].sort((a, b) => a - b);
    memo.set(id, result);
    return result;
  };
  const slots = new Set<number>();
  const identities: Array<readonly [string, number]> = [];
  for (const root of roots) {
    const dependencies = visit(root);
    if (dependencies === null) return null;
    let identity: number | null = null;
    if (dependencies.length === 0) {
      index.staticIdentities ??= staticSubtreeIdentities(frame.template.draw, visit);
      identity = index.staticIdentities[root] ?? null;
    }
    identities.push(identity === null ? ["node", root] : ["static", identity]);
    for (const slot of dependencies) slots.add(slot);
  }
  const view = new DataView(frame.bytes.buffer, frame.bytes.byteOffset, frame.bytes.byteLength);
  const parameters = [...slots]
    .sort((a, b) => a - b)
    .map((slot) => [slot, ...[0, 4, 8, 12].map((lane) => view.getUint32(slot * 16 + lane, true))]);
  return JSON.stringify(["paint-subtree@2", [...frame.template.digest], identities, parameters]);
}

export async function paintDigest(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)));
  return `sha256:${Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

export async function decodePaintTemplate(
  bytes: Uint8Array,
  expected: string,
  cache?: PathArenaCache,
): Promise<PaintTemplateWire> {
  const fail = () => {
    throw new PackedDrawProgramError("invalid paint template");
  };
  if (
    bytes.length < 16 ||
    bytes.length > DRAW_PROGRAM_ABI.maximumBytes + 16 + PAINT_TEMPLATE_ABI.maximumSlots * 8 ||
    new TextDecoder().decode(bytes.subarray(0, 8)) !== PAINT_TEMPLATE_ABI.magic
  )
    fail();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const slots = view.getUint32(8, true);
  const count = view.getUint32(12, true);
  if (
    slots > PAINT_TEMPLATE_ABI.maximumSlots ||
    count > PAINT_TEMPLATE_ABI.maximumSlots ||
    16 + count * 8 >= bytes.length
  )
    fail();
  const actual = await paintDigest(bytes);
  if (actual !== expected) fail();
  const draw = await decodeDrawProgram(bytes.subarray(16 + count * 8), cache);
  const bindings: Array<readonly [number, number]> = [];
  for (let index = 0; index < count; index++) {
    const paint = view.getUint32(16 + index * 8, true);
    const slot = view.getUint32(20 + index * 8, true);
    const source = draw.paints[paint];
    if (
      slot >= slots ||
      source?.kind !== "solid" ||
      (index > 0 && bindings[index - 1]![0] >= paint)
    )
      fail();
    const color = source!.value as Record<string, number>;
    if (
      !color ||
      ![color.red, color.green, color.blue, color.alpha].every((value) => Object.is(value, 0))
    )
      fail();
    bindings.push([paint, slot]);
  }
  const digest = Uint8Array.from(actual.slice(7).match(/../g)!, (byte) =>
    Number.parseInt(byte, 16),
  );
  return { draw, digest, slots, bindings };
}

export async function bindPaintParameters(
  template: PaintTemplateWire,
  bytes: Uint8Array,
  expected: string,
): Promise<DrawProgramWire> {
  const fail = () => {
    throw new PackedDrawProgramError("invalid paint parameters or frame digest");
  };
  if (bytes.length !== template.slots * 16) fail();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const colors: TaggedWire[] = [];
  for (let slot = 0; slot < template.slots; slot++) {
    const offset = slot * 16;
    const [red, green, blue, alpha] = [0, 4, 8, 12].map((lane) =>
      view.getFloat32(offset + lane, true),
    );
    if (
      ![red, green, blue, alpha].every(Number.isFinite) ||
      alpha! < 0 ||
      alpha! > 1 ||
      (alpha === 0 && (red !== 0 || green !== 0 || blue !== 0))
    )
      fail();
    colors.push({ kind: "solid", value: { red, green, blue, alpha } });
  }
  const domain = new TextEncoder().encode("valle.paint-frame@1\0");
  const identity = new Uint8Array(domain.length + 32 + bytes.length);
  identity.set(domain);
  identity.set(template.digest, domain.length);
  identity.set(bytes, domain.length + 32);
  if ((await paintDigest(identity)) !== expected) fail();
  const paintOverrides = new Map<number, TaggedWire>();
  for (const [paint, slot] of template.bindings) paintOverrides.set(paint, colors[slot]!);
  const draw = { ...template.draw, paintOverrides };
  // Own the admitted bits: transport buffers may be reused after this function returns.
  parameterFrames.set(draw, { template, bytes: bytes.slice() });
  return draw;
}
