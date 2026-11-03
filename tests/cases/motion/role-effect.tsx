export const composition = { width: 160, height: 90, fps: 4, duration: 1 };

// An effect reads the picture it works on as input://source: alone, a placeholder of the
// composition's size. A shader shifts its channels apart and darkens its corners; a thumbnail
// of it turns in a corner.
export const role = effect();

const split = `#version 300 es
precision highp float;
uniform sampler2D source;
uniform float amount;
in vec2 uv;
out vec4 color;
void main() {
  float r = texture(source, uv + vec2(amount, 0.0)).r;
  vec2 ga = texture(source, uv).ga;
  float b = texture(source, uv - vec2(amount, 0.0)).b;
  float edge = smoothstep(0.9, 0.3, length(uv - 0.5));
  color = vec4(vec3(r, ga.x, b) * edge, ga.y);
}`;

export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 160, height: 90 }}>
      <ShaderLayer
        code={split}
        inputs={{ source: "input://source" }}
        uniforms={{ amount: 0.01 + seconds * 0.02 }}
        style={{ position: "absolute", inset: 0 }}
      />
      <Image
        src="input://source"
        style={{
          position: "absolute",
          right: 8,
          top: 8,
          width: 48,
          height: 27,
          border: "2px solid white",
          transform: `rotate(${seconds * 20}deg)`,
        }}
      />
    </View>
  );
}
