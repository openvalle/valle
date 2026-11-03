#version 300 es
precision highp float;

// Bands of color that wave along x as time goes on.
uniform float time;
uniform vec3 low;
uniform vec3 high;
in vec2 uv;
out vec4 color;

void main() {
  float y = uv.y + 0.08 * sin(uv.x * 9.0 + time);
  float band = 0.5 + 0.5 * sin(y * 18.0);
  color = vec4(mix(low, high, band), 1.0);
}
