// Copyright 2012 Google LLC. Coverage rules adapted from Skia (BSD-3-Clause).
// See crates/valle-draw/licenses/Skia-BSD.txt.
import type { Canvas, CanvasKit, Image, Surface } from "canvaskit-wasm";
import type { DrawProgramWire, LinearColorWire } from "../canvaskit/draw-program.ts";
import { preserveState, type CanvasKitGpuHost } from "./gpu-state.ts";

interface CircleGroup {
  clip?: unknown;
  mask?: unknown;
  backdrop?: unknown;
  shader?: unknown;
  transition?: unknown;
  filters: readonly unknown[];
  internalBlend: string;
  children: readonly number[];
  transform: readonly number[];
  opacity: number;
}

interface CirclePath {
  stroke?: unknown;
  fill?: number | null;
  path: number;
}

export interface CircleBatch {
  shape: { kind: string; value?: unknown };
  instances: { start: number; count: number };
  pathStyle?: { fill?: boolean } | null;
}

interface CircleRow extends Array<number> {
  cubic?: boolean;
}

interface CircleSlot {
  width: number;
  height: number;
  surface?: Surface;
  image: Image | null;
  fbo?: WebGLFramebuffer | null;
}

interface CircleOptions {
  f16?: boolean;
  roi?: boolean;
  nativeInstances?: boolean;
  pool?: boolean;
}

interface CircleStats {
  precision: "F16" | "RGBA8";
  roi: boolean;
  batches: number;
  circles: number;
  allocatedBytes: number;
  extractionMs: number;
  recordMs: number;
  instanceCircles?: number;
  clearFlushMs?: number;
  stateReadMs?: number;
  glDrawMs?: number;
  stateRestoreMs?: number;
  snapshotMs?: number;
  compositeMs?: number;
}

type TimingStage =
  "clearFlushMs" | "stateReadMs" | "glDrawMs" | "stateRestoreMs" | "snapshotMs" | "compositeMs";
type CircleHost = CanvasKitGpuHost & {
  /** Required only for the RGBA8 external-texture path (f16: false). */
  surface?: Surface;
};
// Real-time GPU geometry consumes admitted DrawPrograms; CPU/reference mode keeps Skia paths.
// It replaces eligible contiguous circle runs, retaining surrounding CanvasKit effects/layers.
const vertex = `#version 300 es
precision highp float;
layout(location=0) in vec4 ellipse;
layout(location=1) in vec4 color;
layout(location=2) in float opacity;
uniform vec2 extent;
uniform vec2 origin;
uniform float bottomLeft;
flat out vec4 vEllipse;
flat out vec4 vColor;
flat out float vOpacity;
void main(){
  vec2 corner=vec2(float(gl_VertexID&1),float((gl_VertexID>>1)&1))*2.-1.;
  vec2 position=ellipse.xy+corner*(ellipse.zw+1.);
  gl_Position=vec4((position-origin)/extent*2.-1.,0.,1.);
  vEllipse=vec4(ellipse.xy-origin,ellipse.zw);vColor=color;vOpacity=opacity;
  if(bottomLeft>0.){gl_Position.y=-gl_Position.y;vEllipse.y=extent.y-vEllipse.y;}
}`;
const fragment = `#version 300 es
precision highp float;
flat in vec4 vEllipse;
flat in vec4 vColor;
flat in float vOpacity;
uniform float cubicPath;
out vec4 result;
float halfRound(float v){
  if(v==0.)return v;
  float m=abs(v)*16384.,unit=0.000000059604644775390625;
  if(m>=65536.){m*=0.0000152587890625;unit*=65536.;}
  if(m>=256.){m*=0.00390625;unit*=256.;}
  if(m>=16.){m*=0.0625;unit*=16.;}
  if(m>=4.){m*=.25;unit*=4.;}
  if(m>=2.)unit*=2.;
  return roundEven(v/unit)*unit;
}
void main(){
  vec2 delta=gl_FragCoord.xy-vEllipse.xy;
  float radius=min(vEllipse.z,vEllipse.w);
  float coverage;
  if(cubicPath>0.){
    vec2 q=abs(delta)/vEllipse.zw;
    float k=.5522847498307936;
    float control=k*vEllipse.z*vEllipse.w/length(vEllipse.zw)<1.?(1.+k)*.5:1.5*k*.5+.5;
    float u=(q.y-q.x+1.)*.5,denominator=2.*control-1.;
    float v=(-control*q.x+(control-1.)*q.y+control)/denominator;
    vec2 du=vec2(halfRound(-.5/vEllipse.z),halfRound(.5/vEllipse.w));
    vec2 dv=vec2(halfRound(-control/denominator/vEllipse.z),halfRound((control-1.)/denominator/vEllipse.w));
    vec2 gradient=vec2(halfRound(2.*u*du.x-dv.x),halfRound(2.*u*du.y-dv.y));
    float edge=halfRound(u*u-v);
    if(q.x+q.y<=1.){
      vec2 n0=normalize(vec2(control*vEllipse.w,(1.-control)*vEllipse.z));
      vec2 n1=normalize(vec2((1.-control)*vEllipse.w,control*vEllipse.z));
      float d0=dot(n0,vec2(vEllipse.z,0.)-abs(delta));
      float d1=dot(n1,vec2(0.,vEllipse.w)-abs(delta));
      coverage=halfRound(min(min(d0,d1)+.5,1.));
    }else coverage=halfRound(clamp(.5-halfRound(edge/max(halfRound(length(gradient)),.000001)),0.,1.));
  }else{
    float outer=radius+.5;
    coverage=halfRound(clamp(outer*(1.-length(delta/outer)),0.,1.));
  }
  if(vOpacity<1.&&cubicPath>0.){
    vec2 minCorner=floor(vEllipse.xy-vEllipse.zw),maxCorner=ceil(vEllipse.xy+vEllipse.zw);
    if(any(lessThan(gl_FragCoord.xy,minCorner))||any(greaterThanEqual(gl_FragCoord.xy,maxCorner)))discard;
  }
  vec4 value=vColor*coverage;
  if(vOpacity<1.)value=vec4(halfRound(value.r),halfRound(value.g),halfRound(value.b),halfRound(value.a))*halfRound(floor(vOpacity*255.+.5)/255.);
  result=value;
}`;

function halfRound(value: number): number {
  if (value === 0) return 0;
  const step = 2 ** Math.max(-24, Math.floor(Math.log2(Math.abs(value))) - 10);
  const v = value / step,
    floor = Math.floor(v),
    rest = v - floor;
  return (floor + Number(rest > 0.5 || (rest === 0.5 && floor % 2 !== 0))) * step;
}
function multiply(a: readonly number[], b: readonly number[]): number[] {
  return a.map(
    (_, i) =>
      a[Math.floor(i / 3) * 3] * b[i % 3] +
      a[Math.floor(i / 3) * 3 + 1] * b[3 + (i % 3)] +
      a[Math.floor(i / 3) * 3 + 2] * b[6 + (i % 3)],
  );
}
function simpleGroup(g: CircleGroup): boolean {
  return (
    !g.clip &&
    !g.mask &&
    !g.backdrop &&
    !g.shader &&
    !g.transition &&
    !g.filters.length &&
    g.internalBlend === "normal"
  );
}

function leaf(
  draw: DrawProgramWire,
  id: number,
  matrix: readonly number[],
  opacity = 1,
  depth = 0,
): CircleRow | null {
  if (depth > 12) return null;
  const tagged = draw.nodes[id];
  if (!tagged) return null;
  if (tagged.kind === "group") {
    const value = tagged.value as CircleGroup;
    if (!simpleGroup(value) || value.children.length !== 1 || (opacity < 1 && value.opacity < 1))
      return null;
    return leaf(
      draw,
      value.children[0],
      multiply(matrix, value.transform),
      opacity * value.opacity,
      depth + 1,
    );
  }
  const value = tagged.value as CirclePath;
  if (tagged.kind !== "path" || value.stroke || value.fill == null) return null;
  const paint = draw.paints[value.fill],
    path = draw.paths[value.path];
  if (
    paint?.kind !== "solid" ||
    !path ||
    path.verbs.join(",") !== "moveTo,cubicTo,cubicTo,cubicTo,cubicTo" ||
    path.points.length !== 13
  )
    return null;
  const pts = path.points,
    cx = (pts[0][0] + pts[6][0]) / 2,
    cy = (pts[3][1] + pts[9][1]) / 2;
  const rx = (pts[0][0] - pts[6][0]) / 2,
    ry = (pts[3][1] - pts[9][1]) / 2;
  if (
    !(rx >= 0.5 && ry >= 0.5 && rx <= 4 && ry <= 4) ||
    matrix[1] !== 0 ||
    matrix[3] !== 0 ||
    matrix[6] !== 0 ||
    matrix[7] !== 0 ||
    matrix[8] !== 1
  )
    return null;
  // Validate all four cubic segments against an axis-aligned ellipse, including control points.
  const k = 0.5522847498307936,
    expected = [
      [1, 0],
      [1, k],
      [k, 1],
      [0, 1],
      [-k, 1],
      [-1, k],
      [-1, 0],
      [-1, -k],
      [-k, -1],
      [0, -1],
      [k, -1],
      [1, -k],
      [1, 0],
    ];
  const tolerance = Math.max(rx, ry) * 1e-5 + 1e-7;
  if (
    pts.some(
      (p, i) =>
        Math.abs(p[0] - (cx + expected[i][0] * rx)) > tolerance ||
        Math.abs(p[1] - (cy + expected[i][1] * ry)) > tolerance,
    )
  )
    return null;
  const c = paint.value as LinearColorWire;
  const row: CircleRow = [
    matrix[0] * cx + matrix[2],
    matrix[4] * cy + matrix[5],
    Math.abs(matrix[0] * rx),
    Math.abs(matrix[4] * ry),
    halfRound(c.red),
    halfRound(c.green),
    halfRound(c.blue),
    c.alpha,
    opacity,
  ];
  row.cubic = true;
  return row;
}

export class GpuCircles {
  declare private readonly host: CircleHost;
  declare private readonly CK: CanvasKit;
  declare private readonly gl: WebGL2RenderingContext;
  declare private readonly f16: boolean;
  declare private readonly roi: boolean;
  declare private readonly nativeInstances: boolean;
  declare private readonly pool: boolean;
  declare private readonly program: WebGLProgram;
  declare private readonly vao: WebGLVertexArrayObject | null;
  declare private readonly buffer: WebGLBuffer | null;
  declare private readonly extent: WebGLUniformLocation | null;
  declare private readonly origin: WebGLUniformLocation | null;
  declare private readonly bottomLeft: WebGLUniformLocation | null;
  declare private readonly cubicPath: WebGLUniformLocation | null;
  declare private slots: CircleSlot[];
  declare private cursor: number;
  declare private closed: boolean | undefined;
  declare readonly stats: CircleStats;

  constructor(
    host: CircleHost,
    { f16 = true, roi = true, nativeInstances = true, pool = true }: CircleOptions = {},
  ) {
    this.host = host;
    this.CK = host.CanvasKit;
    this.gl = host.gl;
    this.f16 = f16;
    this.roi = roi;
    this.nativeInstances = nativeInstances;
    this.pool = pool;
    if (!this.gl) throw new Error("WebGL2 GPU target unavailable");
    const gl = this.gl;
    const restore = preserveState(gl);
    const shader = (type: number, source: string): WebGLShader => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, source);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s)!);
      return s;
    };
    this.program = gl.createProgram()!;
    const vs = shader(gl.VERTEX_SHADER, vertex),
      fs = shader(gl.FRAGMENT_SHADER, fragment);
    gl.attachShader(this.program, vs);
    gl.attachShader(this.program, fs);
    gl.linkProgram(this.program);
    if (!gl.getProgramParameter(this.program, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(this.program)!);
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    this.vao = gl.createVertexArray();
    this.buffer = gl.createBuffer();
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    for (const [i, size, offset] of [
      [0, 4, 0],
      [1, 4, 16],
      [2, 1, 32],
    ]) {
      gl.enableVertexAttribArray(i);
      gl.vertexAttribPointer(i, size, gl.FLOAT, false, 36, offset);
      gl.vertexAttribDivisor(i, 1);
    }
    this.extent = gl.getUniformLocation(this.program, "extent");
    this.origin = gl.getUniformLocation(this.program, "origin");
    this.bottomLeft = gl.getUniformLocation(this.program, "bottomLeft");
    this.cubicPath = gl.getUniformLocation(this.program, "cubicPath");
    this.slots = [];
    this.stats = {
      precision: f16 ? "F16" : "RGBA8",
      roi,
      batches: 0,
      circles: 0,
      extractionMs: 0,
      recordMs: 0,
      allocatedBytes: 0,
    };
    this.beginFrame();
    restore();
    host.resetContext();
  }
  private time(stage: TimingStage, start: number): void {
    this.stats[stage] = (this.stats[stage] ?? 0) + performance.now() - start;
  }
  beginFrame(): void {
    this.cursor = 0;
  }
  endFrame(): void {
    for (const slot of this.slots.splice(this.cursor)) {
      slot.image?.delete();
      slot.surface?.delete();
      this.stats.allocatedBytes -= slot.width * slot.height * 8;
    }
  }
  canCrop(
    draw: DrawProgramWire,
    roots: readonly number[],
    matrix: readonly number[] = [1, 0, 0, 0, 1, 0, 0, 0, 1],
  ): boolean {
    const unit = [
      [1, 0],
      [1, 0.5522847498307936],
      [0.5522847498307936, 1],
      [0, 1],
      [-0.5522847498307936, 1],
      [-1, 0.5522847498307936],
      [-1, 0],
      [-1, -0.5522847498307936],
      [-0.5522847498307936, -1],
      [0, -1],
      [0.5522847498307936, -1],
      [1, -0.5522847498307936],
      [1, 0],
    ];
    const walk = (id: number, total: readonly number[], depth = 0): boolean => {
      if (depth > 12) return false;
      const n = draw.nodes[id];
      if (n?.kind === "group") {
        const v = n.value as CircleGroup;
        return (
          simpleGroup(v) &&
          v.children.every((id) => walk(id, multiply(total, v.transform), depth + 1))
        );
      }
      const v = n?.value as CircleBatch;
      if (
        n?.kind !== "instanceBatch" ||
        v.instances.count < 64 ||
        !["circle", "path"].includes(v.shape.kind)
      )
        return false;
      if (v.pathStyle && v.pathStyle.fill !== true) return false;
      if (v.shape.kind === "path") {
        const path = draw.paths[v.shape.value as number];
        if (!(
          path?.points.length === 13 &&
          path.verbs.join(",") === "moveTo,cubicTo,cubicTo,cubicTo,cubicTo" &&
          path.points.every((p, i) => p.every((v, j) => Math.abs(v - unit[i][j]) < 1e-12))
        ))
          return false;
      }
      // Cropping a raster pass is safe only if every row can use the GPU coverage path.
      // A stroked, skewed or oversized row would fall back to Skia and change AA at the ROI edge.
      const { view, count } = draw.batchInstances;
      if (
        v.instances.start < 0 ||
        v.instances.start + v.instances.count > count ||
        total[6] !== 0 ||
        total[7] !== 0 ||
        total[8] !== 1
      )
        return false;
      for (let i = v.instances.start; i < v.instances.start + v.instances.count; i++) {
        const offset = i * 48,
          a = view.getFloat64(offset, true),
          b = view.getFloat64(offset + 8, true),
          c = view.getFloat64(offset + 16, true),
          d = view.getFloat64(offset + 24, true);
        const aa = total[0] * a + total[1] * b,
          bb = total[3] * a + total[4] * b,
          cc = total[0] * c + total[1] * d,
          dd = total[3] * c + total[4] * d,
          rx = Math.hypot(aa, cc),
          ry = Math.hypot(bb, dd),
          off = aa * bb + cc * dd;
        if (
          !(rx >= 0.5 && rx <= 4 && ry >= 0.5 && ry <= 4 && Math.abs(off) <= rx * ry * 1e-5) ||
          view.getFloat32(count * 68 + i * 4, true) !== 0
        )
          return false;
      }
      return true;
    };
    return Array.isArray(roots) && roots.length > 0 && roots.every((id) => walk(id, matrix));
  }
  draw(
    CK: CanvasKit,
    canvas: Canvas,
    draw: DrawProgramWire,
    roots: readonly number[],
    matrix: readonly number[],
  ): boolean {
    if (roots.length < 64) return false;
    const started = performance.now(),
      total = multiply(canvas.getTotalMatrix(), matrix),
      rows = [];
    for (const id of roots) {
      const row = leaf(draw, id, total);
      if (!row) return false;
      rows.push(row);
    }
    const [x0, y0, x1, y1] = canvas.getDeviceClipBounds(),
      width = x1 - x0,
      height = y1 - y0;
    if (width < 1 || height < 1 || width > 8192 || height > 8192) return false;
    this.stats.extractionMs += performance.now() - started;
    return this.drawRows(
      CK,
      canvas,
      rows.filter((row) => row[7] > 0 && row[8] > 0),
    );
  }
  drawChildren(
    CK: CanvasKit,
    canvas: Canvas,
    draw: DrawProgramWire,
    roots: readonly number[],
    matrix: readonly number[],
    fallback: (node: number) => void,
  ): void {
    const total = multiply(canvas.getTotalMatrix(), matrix);
    let i = 0;
    while (i < roots.length) {
      const rows = [];
      let end = i;
      for (; end < roots.length; end++) {
        const row = leaf(draw, roots[end], total);
        if (!row) break;
        rows.push(row);
      }
      if (
        rows.length >= 64 &&
        this.drawRows(
          CK,
          canvas,
          rows.filter((r) => r[7] > 0 && r[8] > 0),
        )
      ) {
        i = end;
        continue;
      }
      const count = Math.max(1, end - i);
      for (let k = 0; k < count; k++) fallback(roots[i + k]);
      i += count;
    }
  }
  batch(CK: CanvasKit, canvas: Canvas, draw: DrawProgramWire, node: CircleBatch): boolean {
    const pathBatch = node.shape.kind === "path";
    if (!pathBatch && !this.nativeInstances) return false;
    if (
      (node.shape.kind !== "circle" && !pathBatch) ||
      node.instances.count < 64 ||
      (node.pathStyle && node.pathStyle.fill !== true)
    )
      return false;
    if (pathBatch) {
      const path = draw.paths[node.shape.value as number],
        k = 0.5522847498307936;
      const unit = [
        [1, 0],
        [1, k],
        [k, 1],
        [0, 1],
        [-k, 1],
        [-1, k],
        [-1, 0],
        [-1, -k],
        [-k, -1],
        [0, -1],
        [k, -1],
        [1, -k],
        [1, 0],
      ];
      if (
        !path ||
        path.verbs.join(",") !== "moveTo,cubicTo,cubicTo,cubicTo,cubicTo" ||
        path.points.length !== 13 ||
        path.points.some((p, i) => p.some((v, j) => Math.abs(v - unit[i][j]) > 1e-12))
      )
        return false;
    }
    const started = performance.now(),
      total = canvas.getTotalMatrix(),
      view = draw.batchInstances.view,
      n = draw.batchInstances.count,
      rows = [];
    for (let i = node.instances.start; i < node.instances.start + node.instances.count; i++) {
      const k = i * 48,
        a = view.getFloat64(k, true),
        b = view.getFloat64(k + 8, true),
        c = view.getFloat64(k + 16, true),
        d = view.getFloat64(k + 24, true),
        e = view.getFloat64(k + 32, true),
        f = view.getFloat64(k + 40, true);
      if (view.getFloat32(n * 68 + i * 4, true) !== 0) return false;
      const aa = total[0] * a + total[1] * b,
        bb = total[3] * a + total[4] * b,
        cc = total[0] * c + total[1] * d,
        dd = total[3] * c + total[4] * d;
      const rx = Math.hypot(aa, cc),
        ry = Math.hypot(bb, dd),
        off = aa * bb + cc * dd;
      if (
        rx < 0.5 ||
        rx > 4 ||
        ry < 0.5 ||
        ry > 4 ||
        Math.abs(off) > rx * ry * 1e-5 ||
        total[6] !== 0 ||
        total[7] !== 0 ||
        total[8] !== 1
      )
        return false;
      const color = n * 48 + i * 16,
        alpha = view.getFloat32(color + 12, true),
        opacity = view.getFloat32(n * 64 + i * 4, true);
      if (alpha <= 0 || opacity <= 0) continue;
      const quantize = pathBatch ? halfRound : (v: number) => v;
      const row: CircleRow = [
        total[0] * e + total[1] * f + total[2],
        total[3] * e + total[4] * f + total[5],
        rx,
        ry,
        quantize(view.getFloat32(color, true)),
        quantize(view.getFloat32(color + 4, true)),
        quantize(view.getFloat32(color + 8, true)),
        alpha,
        opacity,
      ];
      row.cubic = pathBatch;
      rows.push(row);
    }
    this.stats.extractionMs += performance.now() - started;
    if (!rows.length) return true;
    this.stats.instanceCircles = (this.stats.instanceCircles ?? 0) + rows.length;
    return this.drawRows(CK, canvas, rows);
  }
  private drawRows(CK: CanvasKit, canvas: Canvas, rows: CircleRow[]): boolean {
    let [x0, y0, x1, y1] = canvas.getDeviceClipBounds();
    if (this.roi) {
      let left = Infinity,
        top = Infinity,
        right = -Infinity,
        bottom = -Infinity;
      for (const [x, y, rx, ry] of rows) {
        const padding = Math.max(rx, ry) / Math.min(rx, ry) + 1;
        left = Math.min(left, x - rx - padding);
        top = Math.min(top, y - ry - padding);
        right = Math.max(right, x + rx + padding);
        bottom = Math.max(bottom, y + ry + padding);
      }
      x0 = Math.max(x0, Math.floor(left));
      y0 = Math.max(y0, Math.floor(top));
      x1 = Math.min(x1, Math.ceil(right));
      y1 = Math.min(y1, Math.ceil(bottom));
      if (x1 <= x0 || y1 <= y0) return true;
    }
    let width = x1 - x0,
      height = y1 - y0;
    if (width < 1 || height < 1 || width > 8192 || height > 8192) return false;
    if (!rows.length) return true;
    if (this.cursor >= 64 || width * height * 8 > 64 * 1024 * 1024) return false;
    const data = new Float32Array(rows.flat());
    const gl = this.gl,
      recordStarted = performance.now();
    let slot: CircleSlot | null | undefined = this.slots[this.cursor++];
    if (this.pool) {
      width = slot && slot.width >= width ? slot.width : Math.ceil(width / 128) * 128;
      height = slot && slot.height >= height ? slot.height : Math.ceil(height / 128) * 128;
    }
    const oldBytes = slot ? slot.width * slot.height * (this.f16 ? 8 : 4) : 0;
    const newBytes = width * height * (this.f16 ? 8 : 4);
    if (
      newBytes > 64 * 1024 * 1024 ||
      this.stats.allocatedBytes - oldBytes + newBytes > 256 * 1024 * 1024
    ) {
      this.cursor--;
      return false;
    }
    if (slot && (slot.width !== width || slot.height !== height)) {
      slot.image?.delete();
      if (slot.surface) slot.surface.delete();
      else gl.deleteFramebuffer(slot.fbo!);
      this.stats.allocatedBytes -= slot.width * slot.height * (this.f16 ? 8 : 4);
      slot = null;
    }
    let restore;
    if (this.f16) {
      if (!slot) {
        const surface = CK.MakeRenderTarget(this.host.context, {
          width,
          height,
          colorType: CK.ColorType.RGBA_F16,
          alphaType: CK.AlphaType.Premul,
          colorSpace: CK.ColorSpace.SRGB,
        });
        if (!surface) throw new Error("CanvasKit F16 batch target allocation failed");
        slot = { width, height, surface, image: null };
        this.slots[this.cursor - 1] = slot;
        this.stats.allocatedBytes += width * height * 8;
      }
      slot.image?.delete();
      slot.image = null;
      // Let CanvasKit manage F16 allocation, image identity and copy-on-write. Rendering into its
      // flushed attachment avoids the external-texture API's hardcoded RGBA8 backend format.
      const clearStarted = performance.now();
      slot.surface!.getCanvas().clear(CK.TRANSPARENT);
      slot.surface!.flush();
      this.time("clearFlushMs", clearStarted);
      slot.fbo = gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING);
      if (!slot.fbo) throw new Error("CanvasKit did not leave its F16 target bound");
      const stateStarted = performance.now();
      restore = preserveState(gl);
      this.time("stateReadMs", stateStarted);
    } else {
      const stateStarted = performance.now();
      restore = preserveState(gl);
      this.time("stateReadMs", stateStarted);
    }
    if (!slot) {
      const texture = gl.createTexture()!;
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, width, height);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      const fbo = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
        throw new Error("batch framebuffer incomplete");
      const image = this.host.surface!.makeImageFromTexture(texture, {
        width,
        height,
        colorType: CK.ColorType.RGBA_8888,
        alphaType: CK.AlphaType.Premul,
        colorSpace: CK.ColorSpace.SRGB,
      });
      if (!image) throw new Error("CanvasKit texture import failed");
      slot = { width, height, fbo, image };
      this.slots[this.cursor - 1] = slot;
      this.stats.allocatedBytes += width * height * 4;
    }
    const drawStarted = performance.now();
    gl.bindFramebuffer(gl.FRAMEBUFFER, slot.fbo!);
    gl.viewport(0, 0, width, height);
    gl.disable(gl.SCISSOR_TEST);
    gl.disable(gl.DEPTH_TEST);
    gl.disable(gl.STENCIL_TEST);
    gl.disable(gl.CULL_FACE);
    gl.colorMask(true, true, true, true);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    gl.blendEquation(gl.FUNC_ADD);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.useProgram(this.program);
    gl.uniform2f(this.extent, width, height);
    gl.uniform2f(this.origin, x0, y0);
    gl.uniform1f(this.bottomLeft, this.f16 ? 1 : 0);
    gl.uniform1f(this.cubicPath, rows[0].cubic ? 1 : 0);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STREAM_DRAW);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, rows.length);
    this.time("glDrawMs", drawStarted);
    const restoreStarted = performance.now();
    restore();
    this.host.resetContext();
    this.time("stateRestoreMs", restoreStarted);
    const snapshotStarted = performance.now();
    if (this.f16) slot.image = slot.surface!.makeImageSnapshot();
    this.time("snapshotMs", snapshotStarted);
    const compositeStarted = performance.now();
    const current = canvas.getTotalMatrix(),
      inverse = CK.Matrix.invert(current);
    if (!inverse) throw new Error("noninvertible batch canvas transform");
    canvas.save();
    canvas.concat(inverse);
    canvas.drawImage(slot.image!, x0, y0);
    canvas.restore();
    this.time("compositeMs", compositeStarted);
    this.stats.batches++;
    this.stats.circles += rows.length;
    this.stats.recordMs += performance.now() - recordStarted;
    return true;
  }
  dispose(): void {
    if (this.closed) return;
    this.closed = true;
    const gl = this.gl;
    // Closing the player already released Ganesh. Its last image/surface can destroy cached GL
    // objects; restoring those dead bindings would produce INVALID_OPERATION during teardown.
    const restore = preserveState(gl);
    for (const slot of this.slots) {
      slot.image?.delete();
      if (slot.surface) slot.surface.delete();
      else gl.deleteFramebuffer(slot.fbo!);
    }
    gl.deleteBuffer(this.buffer);
    gl.deleteVertexArray(this.vao);
    gl.deleteProgram(this.program);
    restore();
    this.host.resetContext();
    this.slots = [];
    this.stats.allocatedBytes = 0;
  }
}
