export const composition = { width: 256, height: 128, fps: 4, duration: 1 };

// Shaders that read what is under their box: a lens that magnifies the stripes and words under
// it as it moves, and a pane that pixelates them and shows its own text over that.
const lens = `#version 300 es
precision highp float;
uniform sampler2D backdrop;
in vec2 uv;
out vec4 color;
void main() {
  vec2 d = uv - 0.5;
  float r = length(d);
  vec4 under = texture(backdrop, 0.5 + d * (0.55 + r));
  float edge = smoothstep(0.5, 0.46, r);
  color = vec4(under.rgb, edge);
}`;

const pixels = `#version 300 es
precision highp float;
uniform sampler2D backdrop;
uniform sampler2D content;
in vec2 uv;
out vec4 color;
void main() {
  vec2 cells = vec2(textureSize(backdrop, 0)) / 6.0;
  vec4 under = texture(backdrop, (floor(uv * cells) + 0.5) / cells);
  vec4 own = texture(content, uv);
  color = vec4(mix(under.rgb, own.rgb, own.a), 1.0);
}`;

export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View
      style={{
        width: 256,
        height: 128,
        backgroundImage: "repeating-linear-gradient(45deg, #ffcc80 0 8px, #4fc3f7 8px 16px)",
      }}
    >
      <Text style={{ position: "absolute", left: 16, top: 20, fontSize: 28, fontWeight: 800 }}>
        Under glass
      </Text>
      <ShaderLayer
        code={lens}
        style={{ position: "absolute", left: 20 + seconds * 80, top: 8, width: 64, height: 64 }}
      />
      <ShaderLayer
        code={pixels}
        style={{ position: "absolute", left: 120, top: 72, width: 120, height: 48, padding: 8 }}
      >
        <Text style={{ fontSize: 20, fontWeight: 700, color: "#ffffff" }}>Pixels</Text>
      </ShaderLayer>
    </View>
  );
}
