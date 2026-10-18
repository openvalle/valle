import type { CanvasKit, Image, Surface } from "canvaskit-wasm";
import { decodeScene3dLightingInput } from "./scene3d-input.ts";
import { supportsScene3dGpu } from "../../runtime/compositor/scene3d-gpu-request.ts";
import { preserveState, type CanvasKitGpuHost } from "./gpu-state.ts";
import { norm, linear } from "./scene3d-math.ts";

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
  | { type: "hemisphere"; skyColor: Color4; groundColor: Color4;
      intensity: number; direction: Vec3 };

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
  lightingInput?: Float32Array;
}

interface Scene3dProgram {
  p: WebGLProgram;
  locations: Map<string, WebGLUniformLocation | null>;
}

interface Scene3dTargets {
  width: number;
  height: number;
  color: WebGLTexture;
  depth: WebGLTexture;
  decoded: WebGLTexture;
  fbo: WebGLFramebuffer | null;
  decodeFbo: WebGLFramebuffer | null;
  textures: WebGLTexture[];
  fbos: (WebGLFramebuffer | null)[];
  bytes: number;
}

interface Scene3dRays {
  width: number;
  height: number;
  columns: WebGLTexture;
  rows: WebGLTexture;
}

interface Scene3dVisibility {
  normal: WebGLTexture;
  ids: WebGLTexture;
  shadow: WebGLTexture;
  fbo: WebGLFramebuffer | null;
  capacityWidth: number;
  capacityHeight: number;
  bytes: number;
  x?: number;
  y?: number;
  w?: number;
  h?: number;
}

interface Scene3dStats {
  renders: number;
  prepareMs: number;
  recordMs: number;
  allocatedBytes: number;
  fallbacks: number;
  visibilityAllocations: number;
}

// GPU fulfillment of admitted Scene3D requests. Semantic animation and geometry admission
// remain in Engine. All output snapshots use CanvasKit F16 ownership and copy-on-write.
const quad = `#version 300 es
precision highp float;precision highp int;void main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.-1.,0.,1.);}`;
const metadataVertex = `#version 300 es
precision highp float;precision highp int;
layout(location=0)in float pixel;layout(location=1)in vec3 normal;layout(location=2)in float depth;
layout(location=3)in float id;layout(location=4)in vec2 visibility;
uniform vec2 extent;flat out vec4 normalDepthValue;flat out float idValue;flat out vec2 visibilityValue;
void main(){float y=floor(pixel/extent.x),x=pixel-y*extent.x;gl_Position=vec4((vec2(x,y)+.5)/extent*2.-1.,0.,1.);gl_PointSize=1.;normalDepthValue=vec4(normal,depth);idValue=id;visibilityValue=visibility;}`;
const metadataFragment = `#version 300 es
precision highp float;precision highp int;flat in vec4 normalDepthValue;flat in float idValue;flat in vec2 visibilityValue;
layout(location=0)out vec4 normalDepthResult;layout(location=1)out float idResult;layout(location=2)out vec2 visibilityResult;
void main(){normalDepthResult=normalDepthValue;idResult=idValue;visibilityResult=visibilityValue;}`;
const colors = `
uniform uint precisionBarrier;
float f32(float v){return uintBitsToFloat(floatBitsToUint(v)^precisionBarrier);}
vec3 f32v(vec3 v){return uintBitsToFloat(floatBitsToUint(v)^uvec3(precisionBarrier));}
vec4 f32c(vec4 v){return uintBitsToFloat(floatBitsToUint(v)^uvec4(precisionBarrier));}
vec3 toLinear(vec3 c){return mix(c/12.92,pow((c+.055)/1.055,vec3(2.4)),step(vec3(.04045),c));}
vec3 toSRGB(vec3 c){c=max(c,vec3(0));return mix(c*12.92,1.055*pow(c,vec3(1./2.4))-.055,step(vec3(.0031308),c));}
vec4 decode(vec4 c){return c.a>0.?vec4(toLinear(clamp(c.rgb/c.a,0.,1.))*c.a,c.a):vec4(0);}
vec4 encode(vec4 c){float a=clamp(c.a,0.,1.);return a>0.?vec4(toSRGB(clamp(c.rgb/a,0.,1.))*a,a):vec4(0);}`;
const pbrFragment = `#version 300 es
precision highp float;precision highp int;vec3 world;uniform highp sampler2D normalDepth;uniform highp sampler2D objectIds;uniform highp sampler2D shadowVisibility;uniform vec2 inputOrigin;uniform int objectId;uniform highp sampler2D rayColumns;uniform highp sampler2D rayRows;uniform vec2 size;uniform float depthScale;layout(location=0)out vec4 result;layout(location=1)out float viewZ;
uniform vec3 eye;uniform vec3 forward;uniform vec3 base;uniform float metallic;uniform float roughness;uniform vec3 emissive;
uniform float exposure;uniform int kind;uniform bool acesEnabled;uniform vec3 ambient;uniform vec3 hemiBase;uniform mat3 hemiDirectional;
uniform int lightCount;uniform vec3 lightDir[2];uniform vec3 lightColor[2];uniform bool shadows;
uniform highp sampler2D dfg;
${colors}
const float PI=3.141592653589793;
vec3 direct(vec3 n,vec3 v,vec3 l,vec2 fab){
 float nl=max(dot(n,l),0.),nv=max(dot(n,v),0.);if(nl==0.||nv==0.)return vec3(0);
 vec3 h=normalize(v+l);float nh=max(dot(n,h),0.),vh=clamp(dot(v,h),0.,1.),a=roughness*roughness,a2=a*a;
 float d=nh*nh*(a2-1.)+1.,D=a2/(PI*d*d),vis=.5/max(nl*sqrt(nv*nv*(1.-a2)+a2)+nv*sqrt(nl*nl*(1.-a2)+a2),1e-8);
 float power=pow(1.-vh,5.),dielectric=.04+.96*power;vec3 f0=mix(vec3(.04),base,metallic),F=f0+(1.-f0)*power;
 vec3 comp=1.+f0*(1./max(fab.x+fab.y,1e-6)-1.);
 return nl*(base*(1.-metallic)*(1.-dielectric)/PI+F*D*vis*comp);
}
vec3 aces(vec3 c){
 c=mat3(.59719,.076,.02840,.35458,.90834,.13383,.04823,.01566,.83777)*(c*exposure/.6);
 c=(c*(c+.0245786)-.000090537)/(c*(.983729*c+.432951)+.238081);
 return clamp(mat3(1.60475,-.10208,-.00327,-.53108,1.10813,-.07276,-.07367,-.00605,1.07602)*c,0.,1.);
}
void main(){
 ivec2 pixel=ivec2(gl_FragCoord.xy),at=ivec2(pixel.x,int(size.y)-1-pixel.y)-ivec2(inputOrigin);
 if(int(texelFetch(objectIds,at,0).r)!=objectId)discard;
 vec4 sampleValue=texelFetch(normalDepth,at,0);float z=sampleValue.a;vec3 nrm=sampleValue.rgb;
 vec3 ray=texelFetch(rayColumns,ivec2(pixel.x,0),0).rgb+texelFetch(rayRows,ivec2(int(size.y)-1-pixel.y,0),0).rgb;
 world=f32v(eye+f32v(ray*z));vec3 n=nrm,v=normalize(-ray),irr=ambient+hemiBase+hemiDirectional*n;
 vec3 radiance=base;vec2 fab=vec2(0);
 if(kind==2){
  fab=texture(dfg,vec2(roughness,clamp(dot(n,v),0.,1.))).rg;float single=.04*fab.x+fab.y,missing=1.-fab.x-fab.y,avg=.04+.96/21.,multi=single*avg/(1.-missing*avg)*missing;
  radiance=base*(1.-metallic)*(1.-single-multi)*irr/PI;
 }else if(kind==1)radiance=base*irr;
 for(int i=0;i<2;i++){if(i>=lightCount)break;float visibilityValue=1.;
  if(shadows)visibilityValue=texelFetch(shadowVisibility,at,0)[i];
  if(kind==2)radiance+=direct(n,v,lightDir[i],fab)*lightColor[i]*visibilityValue;
  if(kind==1)radiance+=base*max(dot(n,lightDir[i]),0.)*lightColor[i]*visibilityValue;
 }
 radiance+=emissive;result=vec4(toSRGB(acesEnabled?aces(radiance):clamp(radiance*exposure,0.,1.)),1.);viewZ=z;gl_FragDepth=z*depthScale;
}`;
const dofFragment = `#version 300 es
precision highp float;precision highp int;out vec4 result;uniform highp sampler2D source;uniform highp sampler2D rawSource;uniform highp sampler2D depths;
uniform vec2 size;uniform vec4 visibleBounds;uniform vec4 nearBounds;uniform float focus;uniform float maxRadius;uniform bool hasDof;
${colors}
const vec2 dirs[8]=vec2[8](vec2(1,0),vec2(.70710677,.70710677),vec2(0,1),vec2(-.70710677,.70710677),vec2(-1,0),vec2(-.70710677,-.70710677),vec2(0,-1),vec2(.70710677,-.70710677));
const float rings[4]=float[4](.125,.25,.5,1.);const float weights[4]=float[4](4.,2.,1.,.5);
bool inside(vec2 p,vec4 b){return p.x>=b.x&&p.y>=b.y&&p.x<b.z&&p.y<b.w;}
ivec2 physical(ivec2 p){return ivec2(p.x,int(size.y)-1-p.y);}
float zAt(ivec2 p){return texelFetch(depths,physical(p),0).r;}
float coc(float z){return min(f32(maxRadius*abs(f32(1.-f32(focus/z)))),maxRadius);}
ivec2 rounded(vec2 p){return ivec2(f32(p.x+sign(p.x)*.4999999701976776),f32(p.y+sign(p.y)*.4999999701976776));}
void main(){
 ivec2 p=ivec2(int(gl_FragCoord.x),int(size.y)-1-int(gl_FragCoord.y));vec4 raw=texelFetch(rawSource,physical(p),0);if(!hasDof){result=raw;return;}
 float z=zAt(p),radius=coc(z),tolerance=max(f32(z*.005),.005);vec4 original=texelFetch(source,physical(p),0),color=original;
 if(radius>=.75){vec4 sum=original;float count=1.;for(int r=0;r<4;r++)for(int d=0;d<8;d++){
  ivec2 q=rounded(vec2(f32(float(p.x)+f32(f32(dirs[d].x*radius)*rings[r])),f32(float(p.y)+f32(f32(dirs[d].y*radius)*rings[r]))));if(!inside(vec2(q),vec4(0,0,size)))continue;float qz=zAt(q);
  if(z<focus&&qz>f32(z+tolerance))continue;if(qz<f32(z-tolerance)&&(qz<=focus||length(vec2(q-p))>coc(qz)))continue;
  sum+=texelFetch(source,physical(q),0);count+=1.;}color=sum/count;}
 if(inside(vec2(p),nearBounds)){vec4 sum=vec4(0);for(int r=0;r<4;r++)for(int d=0;d<8;d++){
  float radiusAt=r==0?1.:r==3?maxRadius:f32(f32(maxRadius*float(r))/3.);ivec2 q=rounded(vec2(f32(float(p.x)+f32(dirs[d].x*radiusAt)),f32(float(p.y)+f32(dirs[d].y*radiusAt))));if(!inside(vec2(q),vec4(0,0,size)))continue;
  float qz=zAt(q);if(qz>=focus||qz>=f32(z-tolerance)||length(vec2(q-p))>coc(qz))continue;sum+=texelFetch(source,physical(q),0)*weights[r];}
  vec4 near=sum/60.;if(near.a>0.)color=near+color*(1.-near.a);}
 result=radius<.75&&all(equal(color,original))?raw:encode(color);
}`;

export class GpuScene3D {
  declare private readonly host: Scene3dGpuHost;
  declare private readonly CK: CanvasKit;
  declare private readonly gl: WebGL2RenderingContext;
  declare private readonly pbr: Scene3dProgram;
  declare private readonly metadata: Scene3dProgram;
  declare private readonly dof: Scene3dProgram;
  declare private readonly decode: Scene3dProgram;
  declare private readonly metadataVao: WebGLVertexArrayObject | null;
  declare private readonly metadataBuffer: WebGLBuffer | null;
  declare private readonly vao: WebGLVertexArrayObject | null;
  declare private readonly dfg: WebGLTexture;
  declare private active: Scene3dProgram;
  declare private target: Scene3dTargets | undefined;
  declare private output: Surface | null | undefined;
  declare private rays: Scene3dRays | undefined;
  declare private visibility: Scene3dVisibility | undefined;
  declare private closed: boolean | undefined;
  declare readonly stats: Scene3dStats;

  constructor(host: Scene3dGpuHost, dfgBytes: ArrayBuffer) {
    this.host = host;
    this.CK = host.CanvasKit;
    this.gl = host.gl;
    const gl = this.gl;
    if (!gl?.getExtension("EXT_color_buffer_float"))
      throw new Error("F16 WebGL2 unavailable");
    gl.getExtension("OES_texture_float_linear");
    const restore = preserveState(gl);
    this.stats = {
      renders: 0,
      prepareMs: 0,
      recordMs: 0,
      allocatedBytes: 0,
      fallbacks: 0,
      visibilityAllocations: 0,
    };
    this.pbr = this.program(quad, pbrFragment);
    this.metadata = this.program(metadataVertex, metadataFragment);
    this.metadataVao = gl.createVertexArray();
    this.metadataBuffer = gl.createBuffer();
    gl.bindVertexArray(this.metadataVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.metadataBuffer);
    for (const [i, n, offset] of [
      [0, 1, 0],
      [1, 3, 4],
      [2, 1, 16],
      [3, 1, 20],
      [4, 2, 24],
    ]) {
      gl.enableVertexAttribArray(i);
      gl.vertexAttribPointer(i, n, gl.FLOAT, false, 32, offset);
    }
    this.dof = this.program(quad, dofFragment);
    this.decode = this.program(
      quad,
      "#version 300 es\nprecision highp float;precision highp int;out vec4 result;uniform highp sampler2D source;" +
        colors +
        "void main(){result=decode(texelFetch(source,ivec2(gl_FragCoord.xy),0));}",
    );
    this.vao = gl.createVertexArray();
    this.dfg = this.texture(
      16,
      16,
      gl.RG16F,
      gl.RG,
      gl.HALF_FLOAT,
      new Uint16Array(dfgBytes),
      true,
    );

    restore();
    host.resetContext();
  }
  accepts(r: Scene3dGpuRequest): boolean {
    return supportsScene3dGpu(r);
  }
  private program(vs: string, fs: string): Scene3dProgram {
    const gl = this.gl,
      p = gl.createProgram()!;
    for (const [type, src] of [
      [gl.VERTEX_SHADER, vs],
      [gl.FRAGMENT_SHADER, fs],
    ] as const) {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS))
        throw new Error(gl.getShaderInfoLog(s)!);
      gl.attachShader(p, s);
      gl.deleteShader(s);
    }
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS))
      throw new Error(gl.getProgramInfoLog(p)!);
    return { p, locations: new Map() };
  }
  private use(program: Scene3dProgram): void {
    this.active = program;
    this.gl.useProgram(program.p);
  }
  private loc(name: string): WebGLUniformLocation | null {
    const p = this.active;
    if (!p.locations.has(name))
      p.locations.set(name, this.gl.getUniformLocation(p.p, name));
    return p.locations.get(name)!;
  }
  private f(name: string, v: number): void {
    this.gl.uniform1f(this.loc(name), v);
  }
  private i(name: string, v: number): void {
    this.gl.uniform1i(this.loc(name), v);
  }
  private v2(name: string, v: Float32List): void {
    this.gl.uniform2fv(this.loc(name), v);
  }
  private v3(name: string, v: Float32List): void {
    this.gl.uniform3fv(this.loc(name), v);
  }
  private v4(name: string, v: Float32List): void {
    this.gl.uniform4fv(this.loc(name), v);
  }
  private m4(name: string, v: Float32List): void {
    this.gl.uniformMatrix4fv(this.loc(name), false, v);
  }
  private m3(name: string, v: Float32List): void {
    this.gl.uniformMatrix3fv(this.loc(name), false, v);
  }
  private tex(name: string, texture: WebGLTexture, unit = 0): void {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0 + unit);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.bindSampler(unit, null);
    this.i(name, unit);
  }
  private texture(
    w: number,
    h: number,
    internal: number,
    format: number,
    type: number,
    data: ArrayBufferView | null = null,
    filter = false,
  ): WebGLTexture {
    const gl = this.gl,
      t = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, internal, w, h, 0, format, type, data);
    gl.texParameteri(
      gl.TEXTURE_2D,
      gl.TEXTURE_MIN_FILTER,
      filter ? gl.LINEAR : gl.NEAREST,
    );
    gl.texParameteri(
      gl.TEXTURE_2D,
      gl.TEXTURE_MAG_FILTER,
      filter ? gl.LINEAR : gl.NEAREST,
    );
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return t;
  }
  private targets(width: number, height: number): void {
    const gl = this.gl;
    if (
      this.target &&
      this.target.width === width &&
      this.target.height === height
    )
      return;
    const restore = preserveState(gl);
    if (this.target) {
      for (const t of this.target.textures) gl.deleteTexture(t);
      for (const f of this.target.fbos) gl.deleteFramebuffer(f);
      this.output!.delete();
      this.stats.allocatedBytes -= this.target.bytes;
    }
    const color = this.texture(
        width,
        height,
        gl.RGBA16F,
        gl.RGBA,
        gl.HALF_FLOAT,
      ),
      depth = this.texture(width, height, gl.R32F, gl.RED, gl.FLOAT),
      zbuffer = this.texture(
        width,
        height,
        gl.DEPTH_COMPONENT32F,
        gl.DEPTH_COMPONENT,
        gl.FLOAT,
      ),
      decoded = this.texture(width, height, gl.RGBA32F, gl.RGBA, gl.FLOAT);
    const fbo = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
    for (const [t, attachment] of [
      [color, gl.COLOR_ATTACHMENT0],
      [depth, gl.COLOR_ATTACHMENT1],
      [zbuffer, gl.DEPTH_ATTACHMENT],
    ] as const)
      gl.framebufferTexture2D(gl.FRAMEBUFFER, attachment, gl.TEXTURE_2D, t, 0);
    gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1]);
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
      throw new Error("F16 scene MRT incomplete");
    const decodeFbo = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, decodeFbo);
    gl.framebufferTexture2D(
      gl.FRAMEBUFFER,
      gl.COLOR_ATTACHMENT0,
      gl.TEXTURE_2D,
      decoded,
      0,
    );
    restore();
    this.host.resetContext();
    this.output = this.CK.MakeRenderTarget(this.host.context, {
      width,
      height,
      colorType: this.CK.ColorType.RGBA_F16,
      alphaType: this.CK.AlphaType.Premul,
      colorSpace: this.CK.ColorSpace.SRGB,
    });
    if (!this.output) throw new Error("F16 scene output failed");
    const bytes = width * height * (8 + 4 + 4 + 16 + 8);
    this.target = {
      width,
      height,
      color,
      depth,
      decoded,
      fbo,
      decodeFbo,
      textures: [color, depth, zbuffer, decoded],
      fbos: [fbo, decodeFbo],
      bytes,
    };
    this.stats.allocatedBytes += bytes;
  }
  async render(request: Scene3dGpuRequest): Promise<Image> {
    if (this.closed) throw new Error("GPU Scene3D renderer is closed");
    const gl = this.gl,
      CK = this.CK,
      { width, height } = request,
      frame = request.frame,
      started = performance.now();
    // CanvasKit owns the output texture, generation changes and COW snapshots. Each returned image
    // has immutable frame identity even while later requests reuse this one working surface.
    this.targets(width, height);
    this.output!.getCanvas().clear(CK.TRANSPARENT);
    this.output!.flush();
    const outputFbo = gl.getParameter(gl.DRAW_FRAMEBUFFER_BINDING);
    if (!outputFbo) throw new Error("F16 scene attachment unavailable");
    const restore = preserveState(gl);
    try {
      const objects = frame.meshes.map((f, i) => ({
        frame: f,
        spec: request.scene.meshes[i],
        id: i + 1,
      }));
      const input = decodeScene3dLightingInput(
        request.lightingInput,
        width,
        height,
        objects.length,
      );
      const { columns, rows } = input;
      if (
        !this.rays ||
        this.rays.width !== width ||
        this.rays.height !== height
      ) {
        if (this.rays) {
          gl.deleteTexture(this.rays.columns);
          gl.deleteTexture(this.rays.rows);
          this.stats.allocatedBytes -=
            (this.rays.width + this.rays.height) * 12;
        }
        this.rays = {
          width,
          height,
          columns: this.texture(width, 1, gl.RGB32F, gl.RGB, gl.FLOAT),
          rows: this.texture(height, 1, gl.RGB32F, gl.RGB, gl.FLOAT),
        };
        this.stats.allocatedBytes += (width + height) * 12;
      }
      gl.bindTexture(gl.TEXTURE_2D, this.rays.columns);
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        0,
        0,
        width,
        1,
        gl.RGB,
        gl.FLOAT,
        columns,
      );
      gl.bindTexture(gl.TEXTURE_2D, this.rays.rows);
      gl.texSubImage2D(
        gl.TEXTURE_2D,
        0,
        0,
        0,
        height,
        1,
        gl.RGB,
        gl.FLOAT,
        rows,
      );
      const { x, y, width: w, height: h, count } = input;
      // Metadata uses integer texelFetch coordinates, so a larger backing preserves the
      // logical ROI. Clear the whole attachment before scatter to retire every prior object ID.
      let visibility = this.visibility;
      if (!visibility || visibility.capacityWidth < w || visibility.capacityHeight < h) {
        if (visibility) {
          gl.deleteTexture(visibility.normal);
          gl.deleteTexture(visibility.ids);
          gl.deleteTexture(visibility.shadow);
          gl.deleteFramebuffer(visibility.fbo);
          this.stats.allocatedBytes -= visibility.bytes;
        }
        const capacityWidth = Math.min(width, Math.ceil(w / 128) * 128),
          capacityHeight = Math.min(height, Math.ceil(h / 128) * 128);
        const normal = this.texture(capacityWidth, capacityHeight, gl.RGBA32F, gl.RGBA, gl.FLOAT),
          ids = this.texture(capacityWidth, capacityHeight, gl.R32F, gl.RED, gl.FLOAT),
          shadow = this.texture(capacityWidth, capacityHeight, gl.RG32F, gl.RG, gl.FLOAT);
        const fbo = gl.createFramebuffer();
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
        for (const [texture, attachment] of [
          [normal, gl.COLOR_ATTACHMENT0], [ids, gl.COLOR_ATTACHMENT1], [shadow, gl.COLOR_ATTACHMENT2],
        ] as const) gl.framebufferTexture2D(gl.FRAMEBUFFER, attachment, gl.TEXTURE_2D, texture, 0);
        gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1, gl.COLOR_ATTACHMENT2]);
        if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
          throw new Error("Visibility framebuffer incomplete");
        visibility = { normal, ids, shadow, fbo, capacityWidth, capacityHeight,
          bytes: capacityWidth * capacityHeight * 28 };
        this.stats.allocatedBytes += visibility.bytes;
        this.stats.visibilityAllocations++;
        this.visibility = visibility;
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, visibility.fbo);
      gl.viewport(0, 0, w, h);
      gl.disable(gl.SCISSOR_TEST);
      gl.disable(gl.DEPTH_TEST);
      gl.disable(gl.BLEND);
      gl.disable(gl.STENCIL_TEST);
      gl.disable(gl.CULL_FACE);
      gl.colorMask(true, true, true, true);
      gl.clearBufferfv(gl.COLOR, 0, [0, 0, 0, 0]);
      gl.clearBufferfv(gl.COLOR, 1, [0, 0, 0, 0]);
      gl.clearBufferfv(gl.COLOR, 2, [1, 1, 0, 0]);
      this.use(this.metadata);
      this.v2("extent", [w, h]);
      gl.bindVertexArray(this.metadataVao);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.metadataBuffer);
      gl.bufferData(gl.ARRAY_BUFFER, input.spans, gl.STREAM_DRAW);
      gl.drawArrays(gl.POINTS, 0, count);
      Object.assign(visibility, { x, y, w, h });
      objects.forEach((o, i) => (o.id = i + 1));
      const directional = frame.lights.filter((l) => l.type === "directional");
      gl.disable(gl.SCISSOR_TEST);
      gl.disable(gl.STENCIL_TEST);
      gl.disable(gl.BLEND);
      gl.colorMask(true, true, true, true);
      gl.depthMask(true);
      gl.clearDepth(1);
      gl.depthFunc(gl.LESS);
      gl.cullFace(gl.BACK);
      gl.frontFace(gl.CCW);

      gl.bindFramebuffer(gl.FRAMEBUFFER, this.target!.fbo);
      gl.viewport(0, 0, width, height);
      gl.clearBufferfv(gl.COLOR, 0, [0, 0, 0, 0]);
      gl.clearBufferfv(gl.COLOR, 1, [frame.camera.far, 0, 0, 0]);
      gl.clear(gl.DEPTH_BUFFER_BIT);
      gl.disable(gl.DEPTH_TEST);
      this.use(this.pbr);
      this.v3("eye", input.eye);
      this.v2("size", [width, height]);
      this.f("depthScale", input.depthScale);
      this.tex("rayColumns", this.rays.columns, 3);
      this.tex("rayRows", this.rays.rows, 4);
      this.tex("normalDepth", this.visibility!.normal, 5);
      this.tex("objectIds", this.visibility!.ids, 6);
      this.tex("shadowVisibility", this.visibility!.shadow, 7);
      this.v2("inputOrigin", [this.visibility!.x!, this.visibility!.y!]);
      this.f("exposure", frame.exposure);
      this.i("acesEnabled", request.scene.pbr?.toneMapping === "aces" ? 1 : 0);
      const ambient = [0, 0, 0],
        hemiBase = [0, 0, 0],
        hemi = new Float32Array(9);
      for (const l of frame.lights) {
        if (l.type === "ambient")
          for (let i = 0; i < 3; i++)
            ambient[i] += linear(l.color[i]) * l.intensity;
        if (l.type === "hemisphere") {
          const direction = norm(l.direction);
          for (let i = 0; i < 3; i++) {
            const sky = linear(l.skyColor[i]) * l.intensity,
              ground = linear(l.groundColor[i]) * l.intensity;
            hemiBase[i] += (sky + ground) * 0.5;
            for (let j = 0; j < 3; j++)
              hemi[j * 3 + i] += direction[j] * (sky - ground) * 0.5;
          }
        }
      }
      this.v3("ambient", ambient);
      this.v3("hemiBase", hemiBase);
      this.m3("hemiDirectional", hemi);
      this.i("lightCount", directional.length);
      this.i("shadows", request.scene.pbr?.shadows ? 1 : 0);
      this.tex("dfg", this.dfg, 2);
      for (let i = 0; i < directional.length; i++) {
        this.v3(`lightDir[${i}]`, norm(directional[i].direction));
        this.v3(
          `lightColor[${i}]`,
          directional[i].color
            .slice(0, 3)
            .map((v) => linear(v) * directional[i].intensity),
        );
      }
      gl.enable(gl.SCISSOR_TEST);
      gl.scissor(x, height - y - h, w, h);
      for (const o of objects) {
        const f = o.frame.material;
        this.v3("base", (f.color ?? [1, 1, 1]).slice(0, 3).map(linear));
        this.f("metallic", f.metallic ?? 1);
        this.f("roughness", Math.max(0.0525, f.roughness ?? 1));
        this.v3(
          "emissive",
          (f.emissive ?? [0, 0, 0])
            .slice(0, 3)
            .map((v) => linear(v) * (f.emissiveIntensity ?? 1)),
        );
        this.i(
          "kind",
          o.spec.material.kind === "unlit"
            ? 0
            : o.spec.material.kind === "lambert"
              ? 1
              : 2,
        );
        this.i("objectId", o.id);
        gl.bindVertexArray(this.vao);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
      const dof = frame.camera.depthOfField,
        margin = Math.ceil(dof?.maxBlurRadius ?? 0) + 2;
      const visible = [
        Math.max(0, x - margin),
        Math.max(0, height - y - h - margin),
        Math.min(width, x + w + margin),
        Math.min(height, height - y + margin),
      ];
      const near = visible;
      gl.disable(gl.CULL_FACE);
      gl.disable(gl.DEPTH_TEST);
      gl.bindVertexArray(this.vao);
      gl.enable(gl.SCISSOR_TEST);
      if (dof && dof.maxBlurRadius > 0) {
        const m = Math.ceil(dof.maxBlurRadius) + 2,
          x0 = Math.max(0, visible[0] - m),
          y0 = Math.max(0, visible[1] - m),
          x1 = Math.min(width, visible[2] + m),
          y1 = Math.min(height, visible[3] + m);
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.target!.decodeFbo);
        gl.scissor(x0, y0, x1 - x0, y1 - y0);
        this.use(this.decode);
        this.tex("source", this.target!.color);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, outputFbo);
      gl.viewport(0, 0, width, height);
      gl.scissor(
        visible[0],
        visible[1],
        Math.max(0, visible[2] - visible[0]),
        Math.max(0, visible[3] - visible[1]),
      );
      this.use(this.dof);
      this.tex("source", this.target!.decoded);
      this.tex("rawSource", this.target!.color, 2);
      this.tex("depths", this.target!.depth, 1);
      this.v2("size", [width, height]);
      this.v4("visibleBounds", [
        visible[0],
        height - visible[3],
        visible[2],
        height - visible[1],
      ]);
      this.v4("nearBounds", [
        near[0],
        height - near[3],
        near[2],
        height - near[1],
      ]);
      this.i("hasDof", dof && dof.maxBlurRadius > 0 ? 1 : 0);
      this.f("focus", dof?.focusDistance ?? 1);
      this.f("maxRadius", dof?.maxBlurRadius ?? 0);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    } finally {
      restore();
      this.host.resetContext();
    }
    const image = this.output!.makeImageSnapshot();
    this.stats.renders++;
    this.stats.recordMs += performance.now() - started;
    return image;
  }
  dispose(): void {
    if (this.closed) return;
    this.closed = true;
    const gl = this.gl;
    if (this.target) {
      for (const texture of this.target.textures) gl.deleteTexture(texture);
      for (const fbo of this.target.fbos) gl.deleteFramebuffer(fbo);
    }
    for (const program of [this.pbr, this.dof, this.decode, this.metadata])
      gl.deleteProgram(program.p);
    if (this.rays) {
      gl.deleteTexture(this.rays.columns);
      gl.deleteTexture(this.rays.rows);
    }
    if (this.visibility) {
      gl.deleteTexture(this.visibility.normal);
      gl.deleteTexture(this.visibility.ids);
      gl.deleteTexture(this.visibility.shadow);
      gl.deleteFramebuffer(this.visibility.fbo);
    }
    gl.deleteBuffer(this.metadataBuffer);
    gl.deleteVertexArray(this.metadataVao);
    gl.deleteVertexArray(this.vao);
    gl.deleteTexture(this.dfg);
    this.output?.delete();
    this.stats.allocatedBytes = 0;
  }
}
