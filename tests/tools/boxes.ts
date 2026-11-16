// Deterministic, font-free layout trees with an independent arithmetic oracle.
// Generate inputs only; browser and Valle baseline acceptance stays manual.
import { mkdir } from "node:fs/promises";
import { join } from "node:path";
import { format } from "prettier";
import { root } from "../../web/tests/browser";

const check = process.argv.includes("--check");
const seeds = Array.from({ length: 1000 }, (_, i) => (0x76a11e00 + i) >>> 0);
const coverage: Record<string, number> = {};
type Style = Record<string, string | number>;
type Node = { key: string; style: Style; children: Node[] };
type Rect = { x: number; y: number; width: number; height: number };
type Box = { key: string; border: Rect; padding: Rect; content: Rect };
const palette = ["#2463a0", "#e0b060", "#40a080", "#c05080", "#7890b0"];
const note = (key: string) => {
  coverage[key] = (coverage[key] ?? 0) + 1;
};
function random(seed: number) {
  return (n: number) => {
    seed ^= seed << 13;
    seed ^= seed >>> 17;
    seed ^= seed << 5;
    return (seed >>> 0) % n;
  };
}
function tree(seed: number, x: number, y: number) {
  const choose = random(seed);
  const boxes: Box[] = [];
  let serial = 0;
  function node(
    x: number,
    y: number,
    w: number,
    h: number,
    depth: number,
    basis?: [number, number],
    parent = "",
  ): Node {
    const key = `s${seed}-n${serial++}`;
    const full = parent ? `${parent}/${key}` : key;
    const border = choose(3),
      padding = depth < 2 ? choose(3) * 2 : 0;
    const edge = border + padding,
      cw = w - edge * 2,
      ch = h - edge * 2;
    const sizing = choose(2) ? "content-box" : "border-box";
    const cssSize = (value: number, room: number) =>
      choose(2)
        ? value
        : `calc(50% ${value < room / 2 ? "-" : "+"} ${Math.abs(value - room / 2)}px)`;
    const style: Style = {
      position: "relative",
      boxSizing: sizing,
      width: basis ? cssSize(sizing === "content-box" ? cw : w, basis[0]) : w,
      height: basis ? cssSize(sizing === "content-box" ? ch : h, basis[1]) : h,
      border: `${border}px solid #17365d`,
      padding,
      background: palette[choose(palette.length)],
    };
    // Root dimensions are explicit border-box sizes; children exercise both box models.
    if (!basis) style.boxSizing = "border-box";
    boxes.push({
      key: full,
      border: { x, y, width: w, height: h },
      padding: { x: x + border, y: y + border, width: w - 2 * border, height: h - 2 * border },
      content: { x: x + edge, y: y + edge, width: cw, height: ch },
    });
    note(`depth:${depth}`);
    note(`box-sizing:${style.boxSizing}`);
    if (depth === 2 || cw < 40 || ch < 32) return { key, style, children: [] };
    const mode = ["row", "column", "grid", "block"][choose(4)];
    const rtl = choose(2) === 1,
      gap = choose(3) * 2;
    const align = ["start", "center", "end"][choose(3)];
    const between = choose(2) === 1;
    const count = mode === "grid" ? 4 : 2;
    Object.assign(style, {
      direction: rtl ? "rtl" : "ltr",
      display: mode === "grid" ? "grid" : mode === "block" ? "block" : "flex",
    });
    if (mode === "grid")
      Object.assign(style, {
        gridTemplateColumns: "1fr 1fr",
        gridTemplateRows: "1fr 1fr",
        gap,
        alignItems: align,
        justifyItems: align,
      });
    else if (mode !== "block")
      Object.assign(style, {
        flexDirection: mode,
        gap,
        alignItems: `flex-${align === "center" ? "start" : align}`,
        justifyContent: between ? "space-between" : "flex-start",
      });
    if (mode !== "grid" && mode !== "block" && align === "center") style.alignItems = "center";
    note(`display:${mode}`);
    note(`direction:${rtl ? "rtl" : "ltr"}`);
    note(`align:${align}`);
    note(`gap:${gap}`);
    const sw = mode === "row" || mode === "grid" ? (cw - gap) / 2 : cw;
    const sh = mode === "column" || mode === "block" || mode === "grid" ? (ch - gap) / 2 : ch;
    // All dimensions stay integral; random reduced cross/main sizes exercise alignment.
    const sizes = Array.from({ length: count }, () => [
      Math.max(8, Math.floor((sw - choose(3) * 2) / 2) * 2),
      Math.max(8, Math.floor((sh - choose(3) * 2) / 2) * 2),
    ]);
    const used = sizes.reduce((sum, size) => sum + size[mode === "row" ? 0 : 1], 0);
    const step =
      between && mode !== "grid" && mode !== "block"
        ? ((mode === "row" ? cw : ch) - used) / (count - 1)
        : gap;
    let cursor = 0;
    const children = sizes.map(([w, h], i) => {
      const factor = align === "start" ? 0 : align === "center" ? 0.5 : 1;
      let dx: number, dy: number;
      if (mode === "grid") {
        dx = (i % 2) * (sw + gap) + (sw - w) * factor;
        if (rtl) dx = cw - dx - w;
        dy = Math.floor(i / 2) * (sh + gap) + (sh - h) * factor;
      } else if (mode === "row") {
        dx = rtl ? cw - cursor - w : cursor;
        dy = (ch - h) * factor;
      } else {
        dx = (cw - w) * (mode === "block" ? 0 : factor);
        if (rtl) dx = cw - dx - w;
        dy = cursor;
      }
      const child = node(x + edge + dx, y + edge + dy, w, h, depth + 1, [cw, ch], full);
      // A percentage grid item resolves against its grid area, not the whole grid.
      if (mode === "grid") {
        const b = boxes.find((b) => b.key.split("/").at(-1) === child.key)!;
        child.style.width = child.style.boxSizing === "content-box" ? b.content.width : w;
        child.style.height = child.style.boxSizing === "content-box" ? b.content.height : h;
      }
      if (mode === "row" || mode === "column") child.style.flexShrink = 0;
      if (mode === "block" && i + 1 < count) child.style.marginBottom = gap;
      cursor += (mode === "row" ? w : h) + step;
      return child;
    });
    return { key, style, children };
  }
  const value = node(x, y, 144, 128, 0);
  Object.assign(value.style, { position: "absolute", left: x, top: y });
  return { value, boxes };
}
const unitless = new Set(["flexShrink", "opacity", "order", "zIndex"]);
const css = (style: Style) =>
  Object.entries(style)
    .map(
      ([key, value]) =>
        `${key.replace(/[A-Z]/g, (x) => "-" + x.toLowerCase())}:${value}${typeof value === "number" && !unitless.has(key) ? "px" : ""}`,
    )
    .join(";");
const html = (n: Node, parent = ""): string => {
  const key = parent ? `${parent}/${n.key}` : n.key;
  return `<div data-key="${key}" style="${css(n.style)}">${n.children.map((c) => html(c, key)).join("")}</div>`;
};
async function put(path: string, source: string, parser: string) {
  const contents = await format(source, { parser, printWidth: 100, trailingComma: "all" });
  const file = join(root, path);
  if (check) {
    if ((await Bun.file(file).text()) !== contents)
      throw new Error(`Generated input differs: ${path}`);
  } else {
    await mkdir(join(file, ".."), { recursive: true });
    await Bun.write(file, contents);
  }
}
for (let batch = 0; batch < 20; batch++) {
  const name = `css/generated/boxes-${String(batch).padStart(2, "0")}`;
  const group = seeds.slice(batch * 50, batch * 50 + 50);
  const trees = group.map((seed, i) =>
    tree(seed, 8 + (i % 10) * 160, 8 + Math.floor(i / 10) * 144),
  );
  const scene = `export const composition={width:1600,height:736,fps:1,duration:1};
const TREES=${JSON.stringify(trees.map((t) => t.value))};
function Box({node}) { return <View key={node.key} style={node.style}>{node.children.map(child=><Box node={child}/>)}</View>; }
export default function Scene(){return <View style={{width:1600,height:736,background:"white"}}>{TREES.map(node=><Box node={node}/>)}</View>;}`;
  await put(`tests/cases/${name}/scene.motion.tsx`, scene, "typescript");
  await put(
    `tests/baselines/browser/${name}/scene.html`,
    `<!doctype html><html lang="en"><meta charset="utf-8"><title>${name}</title><style>body{margin:0}*{box-sizing:border-box}#canvas{position:relative;width:1600px;height:736px;background:white}</style><div id="canvas">${trees.map((t) => html(t.value)).join("")}</div></html>`,
    "html",
  );
  const config = {
    spec: "https://www.w3.org/TR/css-flexbox-1/#layout-algorithm",
    assertion: `50 fixed-seed trees (${group[0]}–${group[49]}) cross nested block/flex/grid, direction, gaps, alignment, percentage/calc sizes and box models. Expected boxes come from the generator's independent arithmetic model.`,
    at: [0],
    browser: { selected: "chrome", reason: "Candidates require review before acceptance." },
    layout: trees.flatMap((t) => t.boxes),
  };
  const path = `tests/cases/${name}/case.json`;
  if (check) {
    const saved = await Bun.file(join(root, path)).json();
    if (
      JSON.stringify(saved.layout) !== JSON.stringify(config.layout) ||
      saved.assertion !== config.assertion
    )
      throw new Error(`Generated expectations differ: ${path}`);
  } else {
    // Preserve an existing manual browser review when regenerating identical cases.
    const file = Bun.file(join(root, path));
    if (await file.exists()) config.browser = (await file.json()).browser;
    await put(path, JSON.stringify(config), "json");
  }
}
await put(
  "tests/cases/css/generated/seeds.json",
  JSON.stringify({ algorithm: "xorshift32", seeds, coverage }),
  "json",
);
console.log(
  `${check ? "Verified" : "Generated"} ${seeds.length} trees in 20 fixtures; browser baselines were not changed.`,
);
