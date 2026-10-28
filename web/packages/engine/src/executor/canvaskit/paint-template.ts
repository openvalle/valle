import { PAINT_TEMPLATE_ABI, DRAW_PROGRAM_ABI } from "../../generated/packed-abi.ts";
import {
  decodeDrawProgram,
  PackedDrawProgramError,
  type DrawProgramWire,
  type TaggedWire,
} from "./draw-program.ts";
import type { PathArenaCache } from "./path-arena-cache.ts";

/** VLPAINT1 is the canonical Rust PaintTemplate encoding; parameter lanes are exact LE f32. */
export interface PaintTemplateWire {
  readonly draw: DrawProgramWire;
  readonly digest: Uint8Array;
  readonly slots: number;
  readonly bindings: ReadonlyArray<readonly [number, number]>;
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
  return { ...template.draw, paintOverrides };
}
