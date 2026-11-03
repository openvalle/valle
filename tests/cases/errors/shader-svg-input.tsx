// error: errors/shader-svg-input.tsx:13:10: <ShaderLayer> inputs icon: an SVG file is not supported yet…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export const controls = {
  resources: { icon: resource({ kind: "image", default: "../resources/icon.svg" }) },
};

const code = `uniform sampler2D icon; in vec2 uv; out vec4 color;
void main() { color = texture(icon, uv); }`;

// A shader reads raster images.
export default function Scene() {
  return <ShaderLayer code={code} inputs={{ icon: "resource://icon" }} style={{ width: 8 }} />;
}
