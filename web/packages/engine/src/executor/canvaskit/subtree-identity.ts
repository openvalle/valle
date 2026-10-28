import type { DrawProgramWire, TaggedWire } from "./draw-program.ts";

/** Exact, cold structural identities within the admitted template namespace. No arithmetic
 * normalization: paint/geometry values and node boundaries retain their original meaning. */
export interface SubtreeIdentity {
  readonly shape: number;
  readonly slots: readonly number[];
}

export function subtreeIdentities(
  draw: DrawProgramWire,
  dependencies: (id: number) => readonly number[] | null,
  bindings: ReadonlyMap<number, number> = new Map(),
): Array<SubtreeIdentity | null> {
  const entries = new Map<string, number>();
  const intern = (domain: string, value: unknown): number => {
    const key = domain + exact(value);
    let id = entries.get(key);
    if (id === undefined) {
      id = entries.size;
      entries.set(key, id);
    }
    return id;
  };
  const paths = draw.paths.map((path) => intern("path", path));
  const paints = draw.paints.map((paint) => intern("paint", paint));
  const parameter = intern("parameter", null);
  for (const paint of bindings.keys()) paints[paint] = parameter;
  const orderedReads = new Map<string, readonly number[]>();
  let readBytes = 0;
  const nodes: Array<SubtreeIdentity | null> = Array.from(
    { length: draw.nodes.length },
    () => null,
  );
  const visited = new Set<number>();
  const visit = (id: number, depth = 0): SubtreeIdentity | null => {
    if (visited.has(id)) return nodes[id] ?? null;
    visited.add(id);
    const reads = dependencies(id);
    if (depth > 128 || reads === null) return null;
    const node = draw.nodes[id];
    if (!node) return null;
    const value = node.value as Record<string, any>;
    const slots: number[] = [];
    let normalized: TaggedWire;
    switch (node.kind) {
      case "path": {
        for (const paint of [value.fill, value.stroke?.paint]) {
          const slot = bindings.get(paint);
          if (slot !== undefined) slots.push(slot);
        }
        const path = paths[value.path];
        const fill = value.fill == null ? value.fill : paints[value.fill];
        const stroke =
          value.stroke == null
            ? value.stroke
            : {
                ...value.stroke,
                paint: paints[value.stroke.paint],
              };
        if (
          path === undefined ||
          (value.fill != null && fill === undefined) ||
          (stroke != null && stroke.paint === undefined)
        )
          return null;
        normalized = { ...node, value: { ...value, path, fill, stroke } };
        break;
      }
      case "group": {
        const children: number[] = [];
        for (const child of value.children as number[]) {
          const identity = visit(child, depth + 1);
          if (identity === null || slots.length + identity.slots.length > 64) return null;
          slots.push(...identity.slots);
          children.push(identity.shape);
        }
        let mask = value.mask;
        if (mask != null) {
          const source = visit(mask.source, depth + 1);
          if (source === null || slots.length + source.slots.length > 64) return null;
          slots.push(...source.slots);
          mask = { ...mask, source: source.shape };
        }
        let clip = value.clip;
        if (clip?.kind === "path") {
          const path = paths[clip.value.path];
          if (path === undefined) return null;
          clip = { ...clip, value: { ...clip.value, path } };
        }
        normalized = { ...node, value: { ...value, children, mask, clip } };
        break;
      }
      case "shadow":
        normalized = node;
        break;
      default:
        // Packed instance tables need their actual column bytes, not their buffer offsets.
        return null;
    }
    const key = JSON.stringify(slots);
    let interned = orderedReads.get(key);
    if (!interned) {
      // Residency bounds only: an oversized key retains the ordinary exact node/slot key.
      if (slots.length > 64 || readBytes + slots.length * 8 > 1024 * 1024) return null;
      orderedReads.set(key, (interned = slots));
      readBytes += slots.length * 8;
    }
    nodes[id] = { shape: intern("node", normalized), slots: interned };
    return nodes[id]!;
  };
  for (let id = 0; id < nodes.length; id++) visit(id);
  // Discard descriptor maps; retain shape IDs and bounded, shared ordered-read lists.
  return nodes;
}

/** Typed tokens keep strings distinct from numbers, including negative zero. Object key
 * order is irrelevant; child arrays stay ordered. Finite numbers use lossless JS spelling. */
function exact(value: unknown): string {
  if (typeof value === "number") return `n${Object.is(value, -0) ? "-0" : value}`;
  if (typeof value === "string") return JSON.stringify(value);
  if (value === null) return "null";
  if (typeof value !== "object") return String(value);
  if (Array.isArray(value)) return `[${value.map(exact).join(",")}]`;
  return `{${Object.entries(value)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, item]) => `${JSON.stringify(key)}:${exact(item)}`)
    .join(",")}}`;
}
