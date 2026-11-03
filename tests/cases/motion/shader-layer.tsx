export const composition = { width: 320, height: 160, fps: 4, duration: 1 };

// Shaders that paint layers: stripes that move with time in a color from a string, rounded by
// the box's corners; a word its shader ripples and tints, the same turned, and a dot whose glow
// paints past its box.
const stripes = `#version 300 es
precision highp float;
uniform float time;
uniform vec3 tint;
in vec2 uv;
out vec4 color;
void main() {
  float s = step(0.5, fract((uv.x + uv.y) * 4.0 - time));
  color = vec4(mix(tint, vec3(1.0), s * 0.6), 1.0);
}`;

const ripple = `#version 300 es
precision highp float;
uniform sampler2D content;
uniform float phase;
uniform vec4 tint;
in vec2 uv;
out vec4 color;
void main() {
  vec2 p = uv + vec2(0.0, 0.04 * sin(uv.x * 12.0 + phase));
  vec4 c = texture(content, p);
  color = vec4(mix(c.rgb, tint.rgb, uv.y * tint.a), c.a);
}`;

const glow = `#version 300 es
precision highp float;
uniform vec3 tint;
in vec2 uv;
out vec4 color;
void main() {
  float d = length(uv - 0.5);
  float a = smoothstep(0.5, 0.25, d) + 0.6 * smoothstep(1.0, 0.25, d);
  color = vec4(tint, clamp(a, 0.0, 1.0));
}`;

export default function Scene() {
  const { seconds } = useCurrentTime();
  const word = { fontSize: 32, fontWeight: 800, color: "#1565c0" };
  return (
    <View
      style={{
        display: "flex",
        flexDirection: "column",
        width: 320,
        height: 160,
        background: "#fffde7",
        padding: 12,
        gap: 12,
      }}
    >
      <View style={{ display: "flex", flexDirection: "row", gap: 12 }}>
        <ShaderLayer
          code={stripes}
          uniforms={{ time: seconds, tint: "#7e57c2" }}
          style={{ width: 96, height: 56, borderRadius: 8 }}
        />
        <ShaderLayer
          code={ripple}
          uniforms={{ phase: seconds * 6, tint: "#e9196380" }}
          style={{ padding: 4 }}
        >
          <Text style={word}>Ripple</Text>
        </ShaderLayer>
      </View>
      <View style={{ display: "flex", flexDirection: "row", gap: 32, alignItems: "center" }}>
        <ShaderLayer
          code={ripple}
          uniforms={{ phase: 1, tint: [0.2, 0.7, 0.3, 0.5] }}
          style={{ padding: 4, rotate: `${-8 + seconds * 16}deg` }}
        >
          <Text style={word}>Turned</Text>
        </ShaderLayer>
        <ShaderLayer
          code={glow}
          outset={16}
          uniforms={{ tint: [1, 0.6, 0.1] }}
          style={{ width: 32, height: 32 }}
        />
      </View>
    </View>
  );
}
