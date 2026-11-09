export const composition = { width: 128, height: 96, fps: 4, duration: 1 };
export const controls = {
  resources: {
    effect: resource({ kind: "shader", default: "./input.wgsl" }),
    grid: resource({ kind: "image", default: "../svg/pixels.png" }),
  },
};
const content = `
@group(2) @binding(2) var content: texture_2d<f32>;
@group(2) @binding(6) var sampling: sampler;
fn shade(input: ShaderInput) -> vec4f {
  return textureSampleLevel(content, sampling, input.uv, 0.0);
}`;
const generated = `
struct Params { phase: f32, tint: vec3f, enabled: u32, matrix: mat2x2f }
@group(2) @binding(7) var<uniform> params: Params;
fn shade(input: ShaderInput) -> vec4f {
  let uv = params.matrix * input.uv;
  let color = vec3f(params.phase, uv.y, 0.0) * params.tint;
  return vec4f(select(vec3f(0.0), color, params.enabled != 0u), 0.5);
}`;
const backdrop = `
@group(2) @binding(1) var backdrop: texture_2d<f32>;
@group(2) @binding(4) var sampling: sampler;
fn shade(input: ShaderInput) -> vec4f {
  let c = textureSampleLevel(backdrop, sampling, input.uv, 0.0);
  return vec4f(1.0 - c.rgb, c.a);
}`;
const transition = `
struct Params { progress: f32 }
@group(2) @binding(0) var<uniform> params: Params;
fn shade(input: ShaderInput) -> vec4f {
  return mix(getFromColor(input.uv), getToColor(input.uv), clamp(params.progress, 0.0, 1.0));
}`;
const box = (left, top, width = 64) => ({ position: "absolute", left, top, width, height: 32 });
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 128, height: 96, background: "black" }}>
      <ShaderLayer code={content} style={box(0, 0)}>
        <View style={{ width: 64, height: 16, background: "red" }} />
        <View style={{ width: 64, height: 16, background: "blue" }} />
      </ShaderLayer>
      <ShaderLayer
        source="resource://effect"
        inputs={{ picture: "resource://grid" }}
        style={box(64, 0)}
      />
      <ShaderLayer
        code={generated}
        uniforms={{ phase: seconds, tint: "white", enabled: true, matrix: [1, 0, 0, 1] }}
        style={box(0, 32)}
      />
      <View style={{ ...box(64, 32), background: "#204080" }} />
      <ShaderLayer code={backdrop} style={box(64, 32)} />
      <Transition code={transition} progress={seconds} style={box(0, 64, 128)}>
        <View style={{ width: "100%", height: "100%", background: "red" }} />
        <View style={{ width: "100%", height: "100%", background: "blue" }} />
      </Transition>
    </View>
  );
}
