import type { CanvasKit, GrDirectContext, Surface } from "canvaskit-wasm";

export interface CanvasKitGpuHost {
  CanvasKit: CanvasKit;
  gl: WebGL2RenderingContext;
  context: GrDirectContext;
  resetContext(): void;
}

/** Pinned CanvasKit 0.42 invalidates texture bindings after host-side GL use. */
export function resetCanvasKitContext(surface: Surface): void {
  const bridge = surface as Surface & { _resetContext(): void };
  if (typeof bridge._resetContext !== "function")
    throw new Error("CanvasKit GL interop ABI is unavailable");
  bridge._resetContext();
}

type State = Record<string, any>;
interface Mirror {
  values: State;
  enabled: Set<number>;
  textures: (WebGLTexture | null)[];
  samplers: (WebGLSampler | null)[];
  deleted: WeakSet<object>;
  restore: (() => void)[];
}
const mirrors = new WeakMap<WebGL2RenderingContext, Mirror>();
const names = [
  "CURRENT_PROGRAM",
  "ARRAY_BUFFER_BINDING",
  "VERTEX_ARRAY_BINDING",
  "DRAW_FRAMEBUFFER_BINDING",
  "READ_FRAMEBUFFER_BINDING",
  "VIEWPORT",
  "COLOR_WRITEMASK",
  "COLOR_CLEAR_VALUE",
  "BLEND_EQUATION_RGB",
  "BLEND_EQUATION_ALPHA",
  "BLEND_SRC_RGB",
  "BLEND_DST_RGB",
  "BLEND_SRC_ALPHA",
  "BLEND_DST_ALPHA",
  "ACTIVE_TEXTURE",
  "SCISSOR_BOX",
  "DEPTH_FUNC",
  "DEPTH_WRITEMASK",
  "DEPTH_CLEAR_VALUE",
  "CULL_FACE_MODE",
  "FRONT_FACE",
] as const;
const switches = [
  "SCISSOR_TEST",
  "DEPTH_TEST",
  "STENCIL_TEST",
  "CULL_FACE",
  "BLEND",
] as const;

/** Mirror only this owned context. CanvasKit and host calls both pass through its methods. */
export function trackGpuState(gl: WebGL2RenderingContext): void {
  if (mirrors.has(gl)) return;
  const values = Object.fromEntries(
      names.map((n) => [n, gl.getParameter(gl[n])]),
    ),
    textures: (WebGLTexture | null)[] = [],
    samplers: (WebGLSampler | null)[] = [];
  const units = gl.getParameter(gl.MAX_COMBINED_TEXTURE_IMAGE_UNITS) as number;
  for (let i = 0; i < units; i++) {
    gl.activeTexture(gl.TEXTURE0 + i);
    textures.push(gl.getParameter(gl.TEXTURE_BINDING_2D));
    samplers.push(gl.getParameter(gl.SAMPLER_BINDING));
  }
  gl.activeTexture(values.ACTIVE_TEXTURE);
  const mirror: Mirror = {
    values,
    textures,
    samplers,
    deleted: new WeakSet(),
    enabled: new Set(
      switches.filter((n) => gl.isEnabled(gl[n])).map((n) => gl[n]),
    ),
    restore: [],
  };
  mirrors.set(gl, mirror);
  const wrap = (name: string, record: (...args: any[]) => void) => {
    const object = gl as unknown as Record<string, any>,
      original = object[name],
      own = Object.getOwnPropertyDescriptor(gl, name);
    const wrapped = function (...args: any[]) {
      const result = original.apply(gl, args);
      record(...args);
      return result;
    };
    Object.defineProperty(gl, name, {
      configurable: true,
      writable: true,
      value: wrapped,
    });
    mirror.restore.push(() => {
      if (own) Object.defineProperty(gl, name, own);
      else delete object[name];
    });
  };
  const scalar: Record<string, string> = {
    useProgram: "CURRENT_PROGRAM",
    bindVertexArray: "VERTEX_ARRAY_BINDING",
    activeTexture: "ACTIVE_TEXTURE",
    depthFunc: "DEPTH_FUNC",
    depthMask: "DEPTH_WRITEMASK",
    clearDepth: "DEPTH_CLEAR_VALUE",
    cullFace: "CULL_FACE_MODE",
    frontFace: "FRONT_FACE",
  };
  for (const [method, key] of Object.entries(scalar))
    wrap(method, (v) => (values[key] = v));
  for (const [method, key] of [
    ["viewport", "VIEWPORT"],
    ["scissor", "SCISSOR_BOX"],
    ["colorMask", "COLOR_WRITEMASK"],
    ["clearColor", "COLOR_CLEAR_VALUE"],
  ])
    wrap(method!, (...v) => (values[key!] = v));
  wrap("bindBuffer", (target, buffer) => {
    if (target === gl.ARRAY_BUFFER) values.ARRAY_BUFFER_BINDING = buffer;
  });
  wrap("bindFramebuffer", (target, buffer) => {
    if (target === gl.FRAMEBUFFER || target === gl.DRAW_FRAMEBUFFER)
      values.DRAW_FRAMEBUFFER_BINDING = buffer;
    if (target === gl.FRAMEBUFFER || target === gl.READ_FRAMEBUFFER)
      values.READ_FRAMEBUFFER_BINDING = buffer;
  });
  wrap("bindTexture", (target, texture) => {
    if (target === gl.TEXTURE_2D)
      textures[values.ACTIVE_TEXTURE - gl.TEXTURE0] = texture;
  });
  wrap("bindSampler", (unit, sampler) => (samplers[unit] = sampler));
  wrap("deleteSampler", (sampler) => {
    if (sampler) mirror.deleted.add(sampler);
    for (let i = 0; i < samplers.length; i++)
      if (samplers[i] === sampler) samplers[i] = null;
  });
  wrap("deleteProgram", (program) => {
    if (program) mirror.deleted.add(program);
    if (values.CURRENT_PROGRAM === program) values.CURRENT_PROGRAM = null;
  });
  wrap("deleteTexture", (texture) => {
    if (texture) mirror.deleted.add(texture);
    for (let i = 0; i < textures.length; i++)
      if (textures[i] === texture) textures[i] = null;
  });
  wrap("deleteBuffer", (buffer) => {
    if (buffer) mirror.deleted.add(buffer);
    if (values.ARRAY_BUFFER_BINDING === buffer)
      values.ARRAY_BUFFER_BINDING = null;
  });
  wrap("deleteVertexArray", (vao) => {
    if (vao) mirror.deleted.add(vao);
    if (values.VERTEX_ARRAY_BINDING === vao) values.VERTEX_ARRAY_BINDING = null;
  });
  wrap("deleteFramebuffer", (fbo) => {
    if (fbo) mirror.deleted.add(fbo);
    if (values.DRAW_FRAMEBUFFER_BINDING === fbo)
      values.DRAW_FRAMEBUFFER_BINDING = null;
    if (values.READ_FRAMEBUFFER_BINDING === fbo)
      values.READ_FRAMEBUFFER_BINDING = null;
  });
  wrap("enable", (v) => mirror.enabled.add(v));
  wrap("disable", (v) => mirror.enabled.delete(v));
  wrap("blendEquation", (v) => {
    values.BLEND_EQUATION_RGB = v;
    values.BLEND_EQUATION_ALPHA = v;
  });
  wrap("blendEquationSeparate", (a, b) => {
    values.BLEND_EQUATION_RGB = a;
    values.BLEND_EQUATION_ALPHA = b;
  });
  wrap("blendFunc", (a, b) => {
    values.BLEND_SRC_RGB = a;
    values.BLEND_SRC_ALPHA = a;
    values.BLEND_DST_RGB = b;
    values.BLEND_DST_ALPHA = b;
  });
  wrap("blendFuncSeparate", (a, b, c, d) => {
    values.BLEND_SRC_RGB = a;
    values.BLEND_DST_RGB = b;
    values.BLEND_SRC_ALPHA = c;
    values.BLEND_DST_ALPHA = d;
  });
}

export function untrackGpuState(gl: WebGL2RenderingContext): void {
  const mirror = mirrors.get(gl);
  if (!mirror) return;
  for (const restore of mirror.restore.reverse()) restore();
  mirrors.delete(gl);
}

/** No synchronous getParameter calls on the hot path. Ganesh keeps its matching GL cache. */
export function preserveState(gl: WebGL2RenderingContext): () => void {
  trackGpuState(gl);
  const mirror = mirrors.get(gl)!,
    v = { ...mirror.values },
    enabled = new Set(mirror.enabled),
    textures = mirror.textures.slice(),
    samplers = mirror.samplers.slice();
  const alive = (value: any) =>
    value && mirror.deleted.has(value) ? null : value;
  return () => {
    gl.useProgram(alive(v.CURRENT_PROGRAM));
    gl.bindVertexArray(alive(v.VERTEX_ARRAY_BINDING));
    gl.bindBuffer(gl.ARRAY_BUFFER, alive(v.ARRAY_BUFFER_BINDING));
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, alive(v.DRAW_FRAMEBUFFER_BINDING));
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, alive(v.READ_FRAMEBUFFER_BINDING));
    gl.viewport(...(v.VIEWPORT as [number, number, number, number]));
    gl.scissor(...(v.SCISSOR_BOX as [number, number, number, number]));
    gl.colorMask(
      ...(v.COLOR_WRITEMASK as [boolean, boolean, boolean, boolean]),
    );
    gl.clearColor(...(v.COLOR_CLEAR_VALUE as [number, number, number, number]));
    gl.depthFunc(v.DEPTH_FUNC);
    gl.depthMask(v.DEPTH_WRITEMASK);
    gl.clearDepth(v.DEPTH_CLEAR_VALUE);
    gl.cullFace(v.CULL_FACE_MODE);
    gl.frontFace(v.FRONT_FACE);
    gl.blendEquationSeparate(v.BLEND_EQUATION_RGB, v.BLEND_EQUATION_ALPHA);
    gl.blendFuncSeparate(
      v.BLEND_SRC_RGB,
      v.BLEND_DST_RGB,
      v.BLEND_SRC_ALPHA,
      v.BLEND_DST_ALPHA,
    );
    for (const n of switches)
      enabled.has(gl[n]) ? gl.enable(gl[n]) : gl.disable(gl[n]);
    for (let i = 0; i < textures.length; i++)
      if (mirror.textures[i] !== textures[i]) {
        gl.activeTexture(gl.TEXTURE0 + i);
        gl.bindTexture(gl.TEXTURE_2D, alive(textures[i]!));
      }
    for (let i = 0; i < samplers.length; i++)
      if (mirror.samplers[i] !== samplers[i])
        gl.bindSampler(i, alive(samplers[i]!));
    gl.activeTexture(v.ACTIVE_TEXTURE);
  };
}
