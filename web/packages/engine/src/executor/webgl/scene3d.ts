import type { CanvasKit, Image, Surface } from "canvaskit-wasm";
import { supportsScene3dGpu } from "../../runtime/compositor/scene3d-gpu-request.ts";
import { preserveState, type CanvasKitGpuHost } from "./gpu-state.ts";
import type { Scene3dGpuPacket, Scene3dGpuTexture } from "./scene3d-input.ts";
import shaders from "./scene3d/shaders.json";
import {
  scene3dCopyFragment,
  scene3dDofFragment,
  scene3dDecodeFragment,
  scene3dClassifyFragment,
  scene3dDilateFragment,
  needsDofTiles,
} from "./scene3d/dof.ts";
export type Scene3dGpuHost = CanvasKitGpuHost;

type Vec3 = readonly [number, number, number];
type Color4 = readonly [number, number, number, number];

interface Scene3dMaterialFrame {
  color?: Color4;
  metallic?: number;
  roughness?: number;
  emissive?: Color4;
  emissiveIntensity?: number;
}

type Scene3dLight =
  | { type: "ambient"; color: Color4; intensity: number }
  | { type: "directional"; color: Color4; intensity: number; direction: Vec3 }
  | {
      type: "hemisphere";
      skyColor: Color4;
      groundColor: Color4;
      intensity: number;
      direction: Vec3;
    };

interface Scene3dMeshSpec {
  geometry?: unknown;
  modelControl?: string;
  nodeIds?: readonly number[];
  animationClip?: number;
  materialOverrides?: readonly unknown[];
  material: {
    kind?: "unlit" | "lambert" | "pbr";
    alphaMode?: "opaque" | "mask" | "blend";
    textures?: Readonly<Record<string, unknown>>;
  };
}

interface Scene3dMeshFrame {
  material: Scene3dMaterialFrame;
  transform: { scale: Vec3 };
  nodes?: readonly unknown[];
  materialOverrides?: readonly unknown[];
}

/** Admitted canonical fields consumed by GPU eligibility and execution. */
export interface Scene3dGpuRequest {
  width: number;
  height: number;
  scene: {
    meshes: readonly Scene3dMeshSpec[];
    pbr?: { environment?: unknown; toneMapping?: "none" | "aces"; shadows?: boolean };
  };
  frame: {
    meshes: readonly Scene3dMeshFrame[];
    lights: readonly Scene3dLight[];
    exposure: number;
    camera: {
      far: number;
      depthOfField?: { focusDistance: number; maxBlurRadius: number } | null;
    };
  };
}

type Program = { program: WebGLProgram; locations: Map<string, WebGLUniformLocation | null> };
const postprocess = {
  copy: scene3dCopyFragment,
  dof: scene3dDofFragment,
  decode: scene3dDecodeFragment,
  classify: scene3dClassifyFragment,
  dilate: scene3dDilateFragment,
};
type ProgramName = keyof typeof shaders | keyof typeof postprocess;
type ShaderStage = {
  source: string;
  textures: Record<string, number>;
  blocks: Record<string, number>;
};
type Targets = {
  key: string;
  color: WebGLTexture;
  depth: WebGLTexture;
  opaque: WebGLFramebuffer;
  resolvedColor?: WebGLTexture;
  resolvedDepth?: WebGLTexture;
  resolved?: WebGLFramebuffer;
  decoded?: WebGLTexture;
  decode?: WebGLFramebuffer;
  classifications?: WebGLTexture;
  classify?: WebGLFramebuffer;
  sharpTiles?: WebGLTexture;
  dilate?: WebGLFramebuffer;
  textures: WebGLTexture[];
  framebuffers: WebGLFramebuffer[];
  bytes: number;
};
const tileBytes = (width: number, height: number, radius: number) =>
  radius > 0 ? Math.ceil(width / 16) * Math.ceil(height / 16) * 16 : 0;
const baseBytes = 1024 + 16 + 32; // DFG, empty texel and two uniform blocks.
const quad = `#version 300 es
precision highp float;precision highp int;void main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.-1.,0.,1.);}`;

/** Full triangle rasterization on the same WebGL2 context as CanvasKit. CPU owns semantic
 * preparation and picking; color, depth, materials, shadows, OIT and DOF stay on the GPU. */
export class GpuScene3D {
  readonly stats = {
    renders: 0,
    prepareMs: 0,
    recordMs: 0,
    allocatedBytes: 0,
    fallbacks: 0,
    atlasUploads: 0,
    meshUploads: 0,
    shadowRenders: 0,
    shadowCacheHits: 0,
    geometryBytes: 0,
    programCompiles: 0,
    compileMs: 0,
  };
  private readonly gl: WebGL2RenderingContext;
  private readonly CK: CanvasKit;
  private readonly programs = new Map<ProgramName, Program>();
  private readonly vao: WebGLVertexArrayObject;
  private readonly globals: WebGLBuffer;
  private readonly shadowRange: WebGLBuffer;
  private readonly dfg: WebGLTexture;
  private readonly empty: WebGLTexture;
  private readonly maxEdge: number;
  private output?: Surface;
  private targets?: Targets;
  private atlas?: { key: string; texture: WebGLTexture; bytes: number };
  private mesh?: { key: string; texture: WebGLTexture; bytes: number };
  private shadows?: {
    key: string;
    texture: WebGLTexture;
    depth: WebGLTexture;
    framebuffer: WebGLFramebuffer;
    bytes: number;
  };
  private closed = false;

  constructor(
    private readonly host: Scene3dGpuHost,
    dfgBytes: ArrayBuffer,
  ) {
    this.gl = host.gl;
    this.CK = host.CanvasKit;
    const gl = this.gl,
      restore = preserveState(gl);
    try {
      this.maxEdge = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number;
      this.vao = gl.createVertexArray()!;
      this.globals = gl.createBuffer()!;
      this.shadowRange = gl.createBuffer()!;
      this.dfg = this.texture(16, 16, gl.RG16F, gl.RG, gl.HALF_FLOAT, new Uint16Array(dfgBytes));
      this.empty = this.texture(1, 1, gl.RGBA32F, gl.RGBA, gl.FLOAT, new Float32Array(4));
      this.stats.allocatedBytes = baseBytes;
    } finally {
      restore();
      host.resetContext();
    }
  }
  accepts(request: Scene3dGpuRequest): boolean {
    return (
      !this.closed &&
      supportsScene3dGpu(request) &&
      request.width <= this.maxEdge &&
      request.height <= this.maxEdge
    );
  }
  acceptsPacket(packet: Scene3dGpuPacket): boolean {
    return (
      [
        packet.vertexData,
        packet.shading,
        packet.references,
        packet.tiles,
        ...packet.shadows,
        ...(packet.lit ? [packet.atlas] : []),
        ...(packet.mesh ? [packet.mesh] : []),
      ].every((t) => t.width <= this.maxEdge && t.height <= this.maxEdge) &&
      packet.shadows.length * 1024 <= this.maxEdge
    );
  }
  /** Peak owned texture/upload storage for admission, before fetching an uncached atlas. */
  workingBytes(packet: Scene3dGpuPacket): number {
    const pixels = packet.width * packet.height;
    const targets =
      pixels * (24 + (packet.blend ? 12 : 0) + (packet.radius > 0 ? 16 : 0)) +
      tileBytes(packet.width, packet.height, packet.radius);
    const atlas = packet.lit ? packet.atlas.bytes : (this.atlas?.bytes ?? 0);
    const shadows = packet.shadows.length
      ? packet.shadows.length * 1024 * 1024 * 8
      : (this.shadows?.bytes ?? 0);
    const atlasUpload = packet.lit && this.atlas?.key !== packet.atlas.key ? packet.atlas.bytes : 0;
    const mesh = packet.mesh?.bytes ?? this.mesh?.bytes ?? 0;
    const meshUpload = packet.mesh && this.mesh?.key !== packet.mesh.key ? packet.mesh.bytes : 0;
    return (
      targets +
      atlas +
      shadows +
      mesh +
      packet.bytes.byteLength * 2 +
      atlasUpload +
      meshUpload +
      pixels * 8 +
      baseBytes
    );
  }
  /** Called inside the render's GL state scope. Unused features never compile, and seeks,
   * target resizes and texture eviction retain the programs owned by this context. */
  private program(name: ProgramName): Program {
    const cached = this.programs.get(name);
    if (cached) return cached;
    const started = performance.now();
    const source = name in shaders ? shaders[name as keyof typeof shaders] : null;
    const p = this.compileProgram(
      source?.vertex.source ?? quad,
      source?.fragment.source ?? postprocess[name as keyof typeof postprocess],
    );
    if (source) {
      const gl = this.gl;
      gl.useProgram(p.program);
      for (const stage of [source.vertex, source.fragment] as ShaderStage[]) {
        for (const [name, unit] of Object.entries(stage.textures))
          gl.uniform1i(this.location(p, name), unit);
        for (const [name, unit] of Object.entries(stage.blocks)) {
          const index = gl.getUniformBlockIndex(p.program, name);
          if (index !== gl.INVALID_INDEX) gl.uniformBlockBinding(p.program, index, unit);
        }
      }
    }
    this.programs.set(name, p);
    this.stats.programCompiles++;
    this.stats.compileMs += performance.now() - started;
    return p;
  }
  private compileProgram(vertex: string, fragment: string): Program {
    const gl = this.gl,
      program = gl.createProgram()!;
    try {
      for (const [type, source] of [
        [gl.VERTEX_SHADER, vertex],
        [gl.FRAGMENT_SHADER, fragment],
      ] as const) {
        const shader = gl.createShader(type)!;
        gl.shaderSource(shader, source);
        gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
          const error = gl.getShaderInfoLog(shader);
          gl.deleteShader(shader);
          throw new Error(`Scene3D shader: ${error}`);
        }
        gl.attachShader(program, shader);
        gl.deleteShader(shader);
      }
      gl.linkProgram(program);
      if (!gl.getProgramParameter(program, gl.LINK_STATUS))
        throw new Error(`Scene3D program: ${gl.getProgramInfoLog(program)}`);
      return { program, locations: new Map() };
    } catch (error) {
      gl.deleteProgram(program);
      throw error;
    }
  }
  private location(program: Program, name: string): WebGLUniformLocation | null {
    if (!program.locations.has(name))
      program.locations.set(name, this.gl.getUniformLocation(program.program, name));
    return program.locations.get(name)!;
  }
  private texture(
    width: number,
    height: number,
    internal: number,
    format: number,
    type: number,
    data: ArrayBufferView | null = null,
  ): WebGLTexture {
    const gl = this.gl,
      texture = gl.createTexture()!;
    // Uploads never overwrite a sampler used by an already configured raster/shadow pass.
    gl.activeTexture(gl.TEXTURE0 + 15);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, width, height, 0, format, type, data);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return texture;
  }
  private upload(input: Scene3dGpuTexture, integer: boolean): WebGLTexture {
    const gl = this.gl;
    return this.texture(
      input.width,
      input.height,
      integer ? gl.RGBA32UI : gl.RGBA32F,
      integer ? gl.RGBA_INTEGER : gl.RGBA,
      integer ? gl.UNSIGNED_INT : gl.FLOAT,
      integer
        ? input.words
        : new Float32Array(input.words.buffer, input.words.byteOffset, input.words.length),
    );
  }
  private bind(texture: WebGLTexture, unit: number): void {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.bindSampler(unit, null);
  }
  private uniform(buffer: WebGLBuffer, unit: number, data: Float32Array): void {
    const gl = this.gl;
    gl.bindBuffer(gl.UNIFORM_BUFFER, buffer);
    gl.bufferData(gl.UNIFORM_BUFFER, data, gl.DYNAMIC_DRAW);
    gl.bindBufferBase(gl.UNIFORM_BUFFER, unit, buffer);
  }
  private framebuffer(
    color: WebGLTexture,
    depth?: WebGLTexture,
    test?: WebGLTexture,
  ): WebGLFramebuffer {
    const gl = this.gl,
      framebuffer = gl.createFramebuffer()!;
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, color, 0);
    if (depth)
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, depth, 0);
    if (test) gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.TEXTURE_2D, test, 0);
    gl.drawBuffers(depth ? [gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1] : [gl.COLOR_ATTACHMENT0]);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
      gl.deleteFramebuffer(framebuffer);
      throw new Error("Scene3D framebuffer is incomplete");
    }
    return framebuffer;
  }
  private clearTargets(): void {
    if (!this.targets) return;
    for (const texture of this.targets.textures) this.gl.deleteTexture(texture);
    for (const framebuffer of this.targets.framebuffers) this.gl.deleteFramebuffer(framebuffer);
    this.stats.allocatedBytes -= this.targets.bytes;
    this.output?.delete();
    this.output = undefined;
    this.targets = undefined;
  }
  private ensureTargets(packet: Scene3dGpuPacket): void {
    const { width, height, blend, radius } = packet,
      gl = this.gl;
    const key = `${width}x${height}:${blend}:${radius > 0}`;
    if (this.targets?.key === key) return;
    this.clearTargets();
    const restore = preserveState(gl);
    const textures: WebGLTexture[] = [],
      framebuffers: WebGLFramebuffer[] = [];
    const allocate = (internal: number, format: number, type: number) => {
      const texture = this.texture(width, height, internal, format, type);
      textures.push(texture);
      return texture;
    };
    let targets: Targets;
    try {
      const color = allocate(gl.RGBA16F, gl.RGBA, gl.HALF_FLOAT),
        depth = allocate(gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE);
      const test = allocate(gl.DEPTH_COMPONENT32F, gl.DEPTH_COMPONENT, gl.FLOAT);
      const opaque = this.framebuffer(color, depth, test);
      framebuffers.push(opaque);
      targets = {
        key,
        color,
        depth,
        opaque,
        textures,
        framebuffers,
        bytes:
          width * height * (24 + (blend ? 12 : 0) + (radius > 0 ? 16 : 0)) +
          tileBytes(width, height, radius),
      };
      if (blend) {
        targets.resolvedColor = allocate(gl.RGBA16F, gl.RGBA, gl.HALF_FLOAT);
        targets.resolvedDepth = allocate(gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE);
        targets.resolved = this.framebuffer(targets.resolvedColor, targets.resolvedDepth);
        framebuffers.push(targets.resolved);
      }
      if (radius > 0) {
        targets.decoded = allocate(gl.RGBA32F, gl.RGBA, gl.FLOAT);
        targets.decode = this.framebuffer(targets.decoded);
        framebuffers.push(targets.decode);
        const tile = () => {
          const value = this.texture(
            Math.ceil(width / 16),
            Math.ceil(height / 16),
            gl.RGBA16F,
            gl.RGBA,
            gl.HALF_FLOAT,
          );
          textures.push(value);
          return value;
        };
        targets.classifications = tile();
        targets.classify = this.framebuffer(targets.classifications);
        targets.sharpTiles = tile();
        targets.dilate = this.framebuffer(targets.sharpTiles);
        framebuffers.push(targets.classify, targets.dilate);
      }
    } catch (error) {
      for (const texture of textures) gl.deleteTexture(texture);
      for (const framebuffer of framebuffers) gl.deleteFramebuffer(framebuffer);
      throw error;
    } finally {
      restore();
      this.host.resetContext();
    }
    this.output =
      this.CK.MakeRenderTarget(this.host.context, {
        width,
        height,
        colorType: this.CK.ColorType.RGBA_F16,
        alphaType: this.CK.AlphaType.Premul,
        colorSpace: this.CK.ColorSpace.SRGB,
      }) ?? undefined;
    if (!this.output) {
      for (const texture of textures) gl.deleteTexture(texture);
      for (const framebuffer of framebuffers) gl.deleteFramebuffer(framebuffer);
      throw new Error("Scene3D F16 output surface is unavailable");
    }
    this.targets = targets;
    this.stats.allocatedBytes += targets.bytes;
  }
  private ensureAtlas(packet: Scene3dGpuPacket, bytes: Uint8Array | null): void {
    if (!packet.lit || this.atlas?.key === packet.atlas.key) return;
    if (!bytes) throw new Error("Scene3D atlas upload is missing");
    if (bytes.byteLength !== packet.atlas.bytes || bytes.byteOffset % 4 !== 0)
      throw new Error("Scene3D atlas upload does not match its admitted extent");
    if (this.atlas) {
      this.gl.deleteTexture(this.atlas.texture);
      this.stats.allocatedBytes -= this.atlas.bytes;
    }
    const texture = this.texture(
      packet.atlas.width,
      packet.atlas.height,
      this.gl.RGBA32F,
      this.gl.RGBA,
      this.gl.FLOAT,
      new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4),
    );
    this.atlas = { key: packet.atlas.key, texture, bytes: bytes.byteLength };
    this.stats.allocatedBytes += bytes.byteLength;
    this.stats.atlasUploads++;
  }
  private renderShadows(packet: Scene3dGpuPacket, temporary: WebGLTexture[]): void {
    const gl = this.gl;
    if (!packet.shadows.length) return;
    if (this.shadows?.key === packet.shadowKey) {
      this.stats.shadowCacheHits++;
      return;
    }
    if (this.shadows) {
      gl.deleteTexture(this.shadows.texture);
      gl.deleteTexture(this.shadows.depth);
      gl.deleteFramebuffer(this.shadows.framebuffer);
      this.stats.allocatedBytes -= this.shadows.bytes;
    }
    const texture = this.texture(1024, 1024 * packet.shadows.length, gl.R32F, gl.RED, gl.FLOAT);
    const depth = this.texture(
      1024,
      1024 * packet.shadows.length,
      gl.DEPTH_COMPONENT32F,
      gl.DEPTH_COMPONENT,
      gl.FLOAT,
    );
    const framebuffer = this.framebuffer(texture, undefined, depth);
    this.shadows = {
      key: packet.shadowKey,
      texture,
      depth,
      framebuffer,
      bytes: 1024 * 1024 * packet.shadows.length * 8,
    };
    this.stats.allocatedBytes += this.shadows.bytes;
    gl.useProgram(this.program("shadow").program);
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.enable(gl.DEPTH_TEST);
    gl.depthMask(true);
    gl.depthFunc(gl.LESS);
    gl.disable(gl.SCISSOR_TEST);
    gl.clearBufferfv(gl.COLOR, 0, new Float32Array([3.4028234663852886e38, 0, 0, 0]));
    gl.clearDepth(1);
    gl.clear(gl.DEPTH_BUFFER_BIT);
    for (const [index, shadow] of packet.shadows.entries()) {
      const vertices = this.upload(shadow, true);
      temporary.push(vertices);
      this.bind(vertices, 6);
      this.uniform(
        this.shadowRange,
        5,
        new Float32Array([shadow.minimum, shadow.scale, index * 1024, 0]),
      );
      gl.viewport(0, index * 1024, 1024, 1024);
      gl.drawArrays(gl.TRIANGLES, 0, shadow.vertices);
      this.stats.shadowRenders++;
    }
  }
  async render(
    packet: Scene3dGpuPacket,
    loadAtlas: () => Promise<Uint8Array>,
    loadMesh?: () => Promise<Uint8Array>,
  ): Promise<Image> {
    if (this.closed) throw new Error("GPU Scene3D renderer is closed");
    const preparationStarted = performance.now(),
      gl = this.gl,
      temporary: WebGLTexture[] = [];
    // Resource fetches yield before taking a GL state snapshot; another player may present meanwhile.
    const atlasBytes =
      packet.lit && this.atlas?.key !== packet.atlas.key ? await loadAtlas() : null;
    const meshBytes = packet.mesh && this.mesh?.key !== packet.mesh.key ? await loadMesh?.() : null;
    this.stats.prepareMs += performance.now() - preparationStarted;
    const started = performance.now();
    this.ensureTargets(packet);
    // Ganesh caches its render-target binding. Restore all host allocations before asking
    // it to clear/flush, then take the interop snapshot after it has bound this output.
    this.output!.getCanvas().clear(this.CK.TRANSPARENT);
    this.output!.flush();
    const outputFramebuffer = gl.getParameter(
      gl.DRAW_FRAMEBUFFER_BINDING,
    ) as WebGLFramebuffer | null;
    if (!outputFramebuffer) throw new Error("Scene3D CanvasKit output attachment is unavailable");
    const restore = preserveState(gl);
    try {
      this.ensureAtlas(packet, atlasBytes);
      if (packet.mesh && this.mesh?.key !== packet.mesh.key) {
        if (
          !meshBytes ||
          meshBytes.byteLength !== packet.mesh.bytes ||
          meshBytes.byteOffset % 4 !== 0
        )
          throw new Error("Scene3D mesh upload does not match its admitted extent");
        if (this.mesh) {
          gl.deleteTexture(this.mesh.texture);
          this.stats.allocatedBytes -= this.mesh.bytes;
        }
        const texture = this.upload(
          {
            ...packet.mesh,
            words: new Uint32Array(
              meshBytes.buffer,
              meshBytes.byteOffset,
              meshBytes.byteLength / 4,
            ),
          },
          true,
        );
        this.mesh = { ...packet.mesh, texture };
        this.stats.allocatedBytes += this.mesh.bytes;
        this.stats.meshUploads++;
      }
      gl.bindVertexArray(this.vao);
      gl.disable(gl.BLEND);
      gl.disable(gl.CULL_FACE);
      // Naga maps WGPU's top-left framebuffer to GL by negating clip Y. Keep
      // gl_FrontFacing consistent with the shared shader's CCW model winding.
      gl.frontFace(gl.CW);
      gl.disable(gl.STENCIL_TEST);
      gl.disable(gl.SCISSOR_TEST);
      gl.colorMask(true, true, true, true);
      this.uniform(
        this.globals,
        0,
        new Float32Array([packet.width, packet.height, packet.depthScale, 0]),
      );
      const vertexData = packet.mesh ? this.mesh!.texture : this.upload(packet.vertexData, true),
        shading = this.upload(packet.shading, false);
      temporary.push(shading);
      if (!packet.mesh) temporary.push(vertexData);
      this.bind(shading, 1);
      this.bind(this.dfg, 2);
      this.bind(this.atlas?.texture ?? this.empty, 3);
      this.renderShadows(packet, temporary);
      this.bind(this.shadows?.texture ?? this.empty, 4);
      this.bind(vertexData, 6);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets!.opaque);
      gl.viewport(0, 0, packet.width, packet.height);
      gl.clearBufferfv(gl.COLOR, 0, new Float32Array([0, 0, 0, 0]));
      gl.clearBufferfv(gl.COLOR, 1, new Float32Array([1, 1, 127 / 255, 127 / 255]));
      gl.depthMask(true);
      gl.clearDepth(0);
      gl.clear(gl.DEPTH_BUFFER_BIT);
      if (packet.background) {
        gl.disable(gl.DEPTH_TEST);
        gl.useProgram(this.program("background").program);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
      gl.enable(gl.DEPTH_TEST);
      gl.depthFunc(packet.mesh ? gl.GEQUAL : gl.GREATER);
      gl.depthMask(true);
      gl.useProgram(this.program(packet.mesh ? "mesh" : packet.lit ? "lit" : "unlit").program);
      gl.drawArrays(gl.TRIANGLES, 0, packet.vertices);
      gl.disable(gl.DEPTH_TEST);
      if (packet.blend) {
        const references = this.upload(packet.references, true),
          tiles = this.upload(packet.tiles, true);
        temporary.push(references, tiles);
        this.bind(references, 8);
        this.bind(tiles, 9);
        this.bind(this.targets!.color, 10);
        this.bind(this.targets!.depth, 11);
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets!.resolved!);
        gl.useProgram(this.program("transparency").program);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
      const color = packet.blend ? this.targets!.resolvedColor! : this.targets!.color;
      const depth = packet.blend ? this.targets!.resolvedDepth! : this.targets!.depth;
      const classifyDof = needsDofTiles(packet);
      if (classifyDof) {
        const gridX = Math.ceil(packet.width / 16),
          gridY = Math.ceil(packet.height / 16);
        const classify = this.program("classify");
        gl.viewport(0, 0, gridX, gridY);
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets!.classify!);
        gl.useProgram(classify.program);
        this.bind(depth, 0);
        gl.uniform1i(this.location(classify, "depths"), 0);
        gl.uniform2f(this.location(classify, "size"), packet.width, packet.height);
        gl.uniform1f(this.location(classify, "focus"), packet.focus);
        gl.uniform1f(this.location(classify, "maxRadius"), packet.radius);
        gl.uniform1ui(this.location(classify, "precisionBarrier"), 0);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        const dilate = this.program("dilate");
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets!.dilate!);
        gl.useProgram(dilate.program);
        this.bind(this.targets!.classifications!, 0);
        gl.uniform1i(this.location(dilate, "classification"), 0);
        gl.uniform2i(this.location(dilate, "grid"), gridX, gridY);
        gl.uniform1i(
          this.location(dilate, "halo"),
          Math.ceil((Math.ceil(Math.max(1, packet.radius)) + 1) / 16),
        );
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.viewport(0, 0, packet.width, packet.height);
      }
      if (packet.radius > 0) {
        const decode = this.program("decode");
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets!.decode!);
        gl.useProgram(decode.program);
        this.bind(color, 0);
        gl.uniform1i(this.location(decode, "source"), 0);
        gl.uniform1ui(this.location(decode, "precisionBarrier"), 0);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
      const output = this.program(packet.radius > 0 ? "dof" : "copy");
      gl.bindFramebuffer(gl.FRAMEBUFFER, outputFramebuffer);
      gl.useProgram(output.program);
      this.bind(packet.radius > 0 ? this.targets!.decoded! : color, 0);
      this.bind(depth, 1);
      this.bind(color, 2);
      if (packet.radius > 0) this.bind(this.targets!.sharpTiles!, 3);
      for (const [name, unit] of [
        ["source", 0],
        ["depths", 1],
        ["rawSource", 2],
        ["sharpTiles", 3],
      ] as const)
        gl.uniform1i(this.location(output, name), unit);
      gl.uniform1ui(this.location(output, "precisionBarrier"), 0);
      gl.uniform2f(this.location(output, "size"), packet.width, packet.height);
      gl.uniform1i(this.location(output, "hasDof"), packet.radius > 0 ? 1 : 0);
      gl.uniform1i(this.location(output, "classifyDof"), classifyDof ? 1 : 0);
      gl.uniform1f(this.location(output, "focus"), packet.focus);
      gl.uniform1f(this.location(output, "maxRadius"), packet.radius);
      const margin = Math.ceil(packet.radius) + 2;
      const bounds = packet.background
        ? [0, 0, packet.width, packet.height]
        : [
            Math.max(0, Math.floor(packet.bounds[0]) - margin),
            Math.max(0, Math.floor(packet.bounds[1]) - margin),
            Math.min(packet.width, Math.ceil(packet.bounds[2]) + margin),
            Math.min(packet.height, Math.ceil(packet.bounds[3]) + margin),
          ];
      gl.uniform4fv(this.location(output, "nearBounds"), bounds);
      gl.enable(gl.SCISSOR_TEST);
      gl.scissor(
        bounds[0]!,
        packet.height - bounds[3]!,
        Math.max(0, bounds[2]! - bounds[0]!),
        Math.max(0, bounds[3]! - bounds[1]!),
      );
      gl.drawArrays(gl.TRIANGLES, 0, 3);
      // CanvasKit's render targets own mip levels. External GL writes bypass Ganesh's
      // dirty tracking, so refresh those levels before the image is sampled at a smaller size.
      const outputTexture = gl.getFramebufferAttachmentParameter(
        gl.FRAMEBUFFER,
        gl.COLOR_ATTACHMENT0,
        gl.FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
      ) as WebGLTexture;
      this.bind(outputTexture, 15);
      gl.generateMipmap(gl.TEXTURE_2D);
      this.stats.renders++;
      this.stats.geometryBytes += packet.bytes.byteLength;
    } finally {
      for (const texture of temporary) gl.deleteTexture(texture);
      restore();
      this.host.resetContext();
    }
    this.stats.recordMs += performance.now() - started;
    return this.output!.makeImageSnapshot();
  }
  trim(): void {
    if (this.mesh) {
      this.gl.deleteTexture(this.mesh.texture);
      this.stats.allocatedBytes -= this.mesh.bytes;
      this.mesh = undefined;
    }
    this.clearTargets();
    if (this.atlas) {
      this.gl.deleteTexture(this.atlas.texture);
      this.stats.allocatedBytes -= this.atlas.bytes;
      this.atlas = undefined;
    }
    if (this.shadows) {
      this.gl.deleteTexture(this.shadows.texture);
      this.gl.deleteTexture(this.shadows.depth);
      this.gl.deleteFramebuffer(this.shadows.framebuffer);
      this.stats.allocatedBytes -= this.shadows.bytes;
      this.shadows = undefined;
    }
    this.host.resetContext();
  }
  dispose(): void {
    if (this.closed) return;
    this.closed = true;
    this.trim();
    for (const program of this.programs.values()) this.gl.deleteProgram(program.program);
    this.programs.clear();
    this.gl.deleteBuffer(this.globals);
    this.gl.deleteBuffer(this.shadowRange);
    this.gl.deleteVertexArray(this.vao);
    this.gl.deleteTexture(this.dfg);
    this.gl.deleteTexture(this.empty);
    this.stats.allocatedBytes = 0;
  }
}
