import type { DrawProgramWire, TaggedWire } from "./draw-program.ts";

/** Exact, cold structural identities within the admitted template namespace. No arithmetic
 * normalization: paint/geometry values and node boundaries retain their original meaning. */
export function staticSubtreeIdentities(
  draw: DrawProgramWire,
  dependencies: (id: number) => readonly number[] | null,
): Array<number | null> {
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
  const nodes: Array<number | null> = Array.from({ length: draw.nodes.length }, () => null);
  const visited = new Set<number>();
  const visit = (id: number, depth = 0): number | null => {
    if (visited.has(id)) return nodes[id] ?? null;
    visited.add(id);
    const reads = dependencies(id);
    if (depth > 128 || reads === null || reads.length !== 0) return null;
    const node = draw.nodes[id];
    if (!node) return null;
    const value = node.value as Record<string, any>;
    let normalized: TaggedWire;
    switch (node.kind) {
      case "path": {
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
        const children = (value.children as number[]).map((child) => visit(child, depth + 1));
        if (children.includes(null)) return null;
        let mask = value.mask;
        if (mask != null) {
          const source = visit(mask.source, depth + 1);
          if (source === null) return null;
          mask = { ...mask, source };
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
    nodes[id] = intern("node", normalized);
    return nodes[id]!;
  };
  for (let id = 0; id < nodes.length; id++) visit(id);
  // Discard descriptor maps; only one nullable number per node remains resident.
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
