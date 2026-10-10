import type { PathWire } from "./draw-program.ts";

export function encodeTestPaths(paths: readonly PathWire[]): Uint8Array {
  const codes = ["moveTo", "lineTo", "quadTo", "cubicTo", "close"];
  const bytes = new Uint8Array(paths.reduce((n, p) => n + 8 + p.verbs.length + p.points.length * 16, 0));
  const view = new DataView(bytes.buffer); let at = 0;
  for (const path of paths) {
    view.setUint32(at, path.verbs.length, true); view.setUint32(at + 4, path.points.length, true); at += 8;
    for (const verb of path.verbs) bytes[at++] = codes.indexOf(verb);
    for (const [x, y] of path.points) {
      view.setFloat64(at, x, true); view.setFloat64(at + 8, y, true); at += 16;
    }
  }
  return bytes;
}
