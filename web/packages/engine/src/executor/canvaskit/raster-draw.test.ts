import { expect, test } from "bun:test";
import type { Canvas, CanvasKit, Path } from "canvaskit-wasm";
import { CanvasKitBuiltinRuntime } from "./builtin-runtime.ts";
import type { DrawProgramWire, PathWire } from "./draw-program.ts";
import { applyClip, CanvasKitExecutor, drawProgramNode, makePaint } from "./executor.ts";
import { programNodeBounds } from "./program-bounds.ts";
import { RasterPathCache } from "./raster-path-cache.ts";
import { PathArenaCache } from "./path-arena-cache.ts";
import { encodeTestPaths } from "./path-arena.test-helper.ts";

const { default: CanvasKitInit } = await import("canvaskit-wasm") as unknown as {
  default: (options: { locateFile(file: string): string }) => Promise<CanvasKit>;
};
const identity = [1, 0, 0, 0, 1, 0, 0, 0, 1];
type Wire = Record<string, any>;

function emptyProgram(): DrawProgramWire {
  const table = { count: 0, bytes: new Uint8Array(), view: new DataView(new ArrayBuffer(0)) };
  return { roots: [], nodes: [], paths: [], paints: [], viewport: { x: 0, y: 0, width: 128, height: 96 },
    batchInstances: table, strokeColors: table, dashOffsets: table, requirements: {} };
}

function group(program: DrawProgramWire, children: number[], options: Wire = {}): number {
  return program.nodes.push({ kind: "group", value: { children, transform: identity, clip: null,
    filters: [], opacity: 1, mask: null, shader: null, transition: null, glass: null,
    glassForeground: null, backdrop: null, internalBlend: "normal", ...options } }) - 1;
}

function path(program: DrawProgramWire, wire: PathWire, fill: number, fillRule = "nonZero"): number {
  return program.nodes.push({ kind: "path", value: {
    path: program.paths.push(wire) - 1, fill, fillRule, stroke: null,
  } }) - 1;
}

/** Reference: ordinary per-verb Skia builder, a fresh Paint and saves around every node. */
function referencePath(CK: CanvasKit, wire: PathWire, fillRule: string): Path {
  const builder = new CK.PathBuilder();
  let index = 0;
  for (const verb of wire.verbs) {
    const a = wire.points[index];
    if (verb === "moveTo") { builder.moveTo(...a!); index++; }
    else if (verb === "lineTo") { builder.lineTo(...a!); index++; }
    else if (verb === "quadTo") {
      const b = wire.points[index + 1]!; builder.quadTo(...a!, ...b); index += 2;
    } else if (verb === "cubicTo") {
      const b = wire.points[index + 1]!, c = wire.points[index + 2]!;
      builder.cubicTo(...a!, ...b, ...c); index += 3;
    } else if (verb === "close") builder.close();
    else throw new Error(`unexpected reference verb ${verb}`);
  }
  builder.setFillType(fillRule === "evenOdd" ? CK.FillType.EvenOdd : CK.FillType.Winding);
  return builder.detachAndDelete();
}

function reference(CK: CanvasKit, canvas: Canvas, program: DrawProgramWire, id: number): void {
  const tagged = program.nodes[id]!, node = tagged.value as Wire;
  canvas.save(); canvas.concat(identity);
  try {
    if (tagged.kind === "path") {
      const geometry = referencePath(CK, program.paths[node.path as number]!, node.fillRule);
      const paint = makePaint(CK, program.paints[node.fill as number]);
      try { canvas.drawPath(geometry, paint); }
      finally { geometry.delete(); paint.delete(); }
      return;
    }
    if (node.opacity === 0) return;
    const paint = node.opacity === 1 ? null : new CK.Paint();
    if (paint) {
      paint.setAlphaf(node.opacity);
      const bounds = programNodeBounds(program, id, new Map());
      canvas.saveLayer(paint, bounds == null ? undefined : CK.XYWHRect(bounds.x, bounds.y, bounds.width, bounds.height));
    }
    canvas.save(); canvas.concat(node.transform);
    let clipped = false;
    try {
      if (node.clip != null) {
        applyClip(CK, canvas, program, node.clip, identity); canvas.saveLayer(); clipped = true;
      }
      for (const child of node.children as number[]) reference(CK, canvas, program, child);
    } finally {
      if (clipped) canvas.restore();
      canvas.restore();
      if (paint) { canvas.restore(); paint.delete(); }
    }
  } finally { canvas.restore(); }
}

test("batched path commands and reused paints preserve overlapping curves, holes, nested opacity and clipped siblings", async () => {
  const CK = await CanvasKitInit({ locateFile: () => Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm", import.meta.dir) });
  const program = emptyProgram();
  program.paints.push(
    { kind: "solid", value: { red: .5, green: .13, blue: .73, alpha: .83 } },
    { kind: "solid", value: { red: .7, green: .93, blue: .27, alpha: 1 } },
    { kind: "linearGradient", value: { start: [0, 0], end: [128, 96], spread: "pad",
      stops: [{ offset: 0, color: { red: .1, green: .3, blue: .9, alpha: 1 } },
        { offset: 1, color: { red: .9, green: .2, blue: .1, alpha: .7 } }] } },
  );
  const curves: number[] = [];
  for (let i = 0; i < 160; i++) {
    const x = (i % 20) * 5 + .37, y = Math.floor(i / 20) * 7 + .93;
    const leaf = path(program, {
      verbs: ["moveTo", "lineTo", "quadTo", "cubicTo", "close", "moveTo", "lineTo", "lineTo", "close"],
      points: [[x, y], [x + 5.7, y], [x + 9.3, y + 3.2], [x + 4.9, y + 6.1],
        [x - 3.4, y + 6.9], [x - 2.7, y - 3.8], [x, y],
        [x + 1.1, y + 1.7], [x + 3.3, y + 1.9], [x + 2.3, y + 3.4]],
    }, i % 3, i % 2 === 0 ? "evenOdd" : "nonZero");
    curves.push(group(program, [leaf], { opacity: [0, .1333333, .75, 1][i % 4] }));
  }
  const clipped = group(program, [group(program, curves, { opacity: .63 })], {
    opacity: .47, transform: [1.25, 0, -8.75, 0, .875, 8.125, 0, 0, 1],
    clip: { kind: "rect", value: { x: 12.17, y: 4.31, width: 87.29, height: 39.51 } },
  });
  const sibling = path(program, { verbs: ["moveTo", "lineTo", "lineTo", "close"],
    points: [[3.27, 73.91], [119.31, 85.17], [83.1, 65.33]] }, 2);
  const root = group(program, [clipped, sibling]);
  const admitted: Parameters<typeof drawProgramNode>[3] = { draw: program, plan: {}, directRaster: null,
    textures: new Map(), fonts: new Map(), shaders: new Map(), scenes: new Map() };
  const builtins = new CanvasKitBuiltinRuntime(CK);
  const info = { width: 128, height: 96, colorType: CK.ColorType.RGBA_F16,
    alphaType: CK.AlphaType.Premul, colorSpace: CK.ColorSpace.SRGB };
  const memory = [CK.Malloc(Uint8Array, 128 * 96 * 8), CK.Malloc(Uint8Array, 128 * 96 * 8)];
  const surfaces = memory.map(mem => CK.MakeRasterDirectSurface(info, mem, 128 * 8)!);
  const Paint = CK.Paint;
  let created = 0;
  try {
    for (const surface of surfaces) surface.getCanvas().clear(CK.TRANSPARENT);
    reference(CK, surfaces[0]!.getCanvas(), program, root);
    Reflect.set(CK, "Paint", function() { created++; return new Paint(); });
    const canvas = surfaces[1]!.getCanvas(), matrix = canvas.getTotalMatrix();
    drawProgramNode(CK, builtins, canvas, admitted, root, identity);
    expect(canvas.getTotalMatrix()).toEqual(matrix);
    expect(created).toBeLessThanOrEqual(program.paints.length + 1);
    const pixels = surfaces.map(surface => surface.getCanvas().readPixels(0, 0, {
      ...info, colorType: CK.ColorType.RGBA_F32,
    }));
    expect(pixels[1]).toEqual(pixels[0]);
  } finally {
    Reflect.set(CK, "Paint", Paint);
    for (const surface of surfaces) surface.delete();
    for (const mem of memory) CK.Free(mem);
    builtins.dispose();
  }
});

test("retained paths preserve f32 curve coverage under changing projective matrices", async () => {
  const CK=await CanvasKitInit({locateFile:()=>Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm",import.meta.dir)});
  const builtins=new CanvasKitBuiltinRuntime(CK),cache=new RasterPathCache(),surfaces=[CK.MakeSurface(128,96)!,CK.MakeSurface(128,96)!];
  const program=emptyProgram();program.paints.push({kind:"solid",value:{red:.4,green:.1,blue:.7,alpha:.8}});
  const root=path(program,{verbs:["moveTo","cubicTo","lineTo","close"],points:[[8,12],[13,53],[66,1],[96,55],[20,80]]},0);
  const admitted={draw:program} as unknown as Parameters<typeof drawProgramNode>[3];
  try {
    for(const matrix of [[1,0,0,0,1,0,0,0,1],[.9,.2,4,-.1,1,9,.002,.001,1],[1,0,0,0,1,0,0,0,1]]){
      for(let at=0;at<2;at++){
        const canvas=surfaces[at]!.getCanvas();canvas.clear(CK.TRANSPARENT);canvas.save();canvas.clipRect(CK.XYWHRect(5,6,95,77),CK.ClipOp.Intersect,true);
        drawProgramNode(CK,builtins,canvas,{...admitted,pathCache:at?cache:undefined},root,matrix);canvas.restore();
      }
      const info={width:128,height:96,colorType:CK.ColorType.RGBA_8888,alphaType:CK.AlphaType.Unpremul,colorSpace:CK.ColorSpace.SRGB};
      expect(surfaces[1]!.getCanvas().readPixels(0,0,info)).toEqual(surfaces[0]!.getCanvas().readPixels(0,0,info));
    }
    expect(cache.hits).toBe(2);expect(cache.misses).toBe(1);
  }finally{cache.dispose();builtins.dispose();surfaces.forEach(s=>s.delete());}
});

test("whole vector scopes preserve projected gradients and nested alpha across warm hits and paint changes", async () => {
  const CK = await CanvasKitInit({locateFile:()=>Bun.resolveSync("canvaskit-wasm/bin/canvaskit.wasm",import.meta.dir)});
  const executor = new CanvasKitExecutor(CK), surfaces = [CK.MakeSurface(128,96)!, CK.MakeSurface(128,96)!];
  const program = emptyProgram();
  program.paints.push({kind:"linearGradient",value:{start:[0,0],end:[128,96],spread:"pad",
    stops:[{offset:0,color:{red:.1,green:.3,blue:.8,alpha:.8}},{offset:1,color:{red:.8,green:.2,blue:.1,alpha:.7}}]}});
  const children: number[] = [];
  for(let i=0;i<12;i++) {
    const x=5+i*7;
    children.push(path(program,{verbs:["moveTo","quadTo","lineTo","close"],points:[[x,5],[x+14,35],[x+3,65],[x+10,80]]},0));
  }
  program.paths = new PathArenaCache().decode(encodeTestPaths(program.paths),program.paths.length);
  const root = group(program,children,{opacity:.67,clip:{kind:"rect",value:{x:4,y:4,width:100,height:80}}});
  const arena = executor.arena(null,128,96,128n*1024n*1024n,128n*1024n*1024n); arena.trees=executor.trees;
  const admitted = {draw:program,pathCache:executor.paths,paintCache:executor.paints} as unknown as Parameters<typeof drawProgramNode>[3];
  try {
    for(const matrix of [identity,identity,[.9,.05,2,-.03,1,4,.001,.0004,1],identity]) {
      arena.treeSignatures=new Map();
      for(let mode=0;mode<2;mode++) {
        const canvas=surfaces[mode]!.getCanvas(); canvas.clear(CK.TRANSPARENT);
        drawProgramNode(CK,executor.builtins,canvas,admitted,root,matrix,0,undefined,mode?arena:undefined);
      }
      const info={width:128,height:96,colorType:CK.ColorType.RGBA_8888,alphaType:CK.AlphaType.Unpremul,colorSpace:CK.ColorSpace.SRGB};
      expect(surfaces[1]!.getCanvas().readPixels(0,0,info)).toEqual(surfaces[0]!.getCanvas().readPixels(0,0,info));
    }
    expect(executor.trees.hits).toBeGreaterThan(0);
    (program.paints[0]!.value as Wire).stops[0].color.red=.7;
    arena.treeSignatures=new Map();
    for(const surface of surfaces)surface.getCanvas().clear(CK.TRANSPARENT);
    drawProgramNode(CK,executor.builtins,surfaces[0]!.getCanvas(),admitted,root,identity);
    drawProgramNode(CK,executor.builtins,surfaces[1]!.getCanvas(),admitted,root,identity,0,undefined,arena);
    const info={width:128,height:96,colorType:CK.ColorType.RGBA_8888,alphaType:CK.AlphaType.Unpremul,colorSpace:CK.ColorSpace.SRGB};
    const expected=surfaces[0]!.getCanvas().readPixels(0,0,info), actual=surfaces[1]!.getCanvas().readPixels(0,0,info);
    expect(actual).toEqual(expected);
  } finally {surfaces.forEach(s=>s.delete());executor.dispose();}
});
