export const composition = { width: 256, height: 96, fps: 4, duration: 1 };

export const controls = {
  resources: {
    photo: resource({ kind: "image", default: "../resources/photo.png" }),
    rings: resource({ kind: "image", default: "../resources/rings.png" }),
    tile: resource({ kind: "image", default: "../resources/tile.png" }),
  },
};

// Shaders that read images by name: the photo as an <Image> shows it, the photo rippled by the
// rings as time goes on, and the tile's pixels read one by one at their own size.
const ripple = `#version 300 es
precision highp float;
uniform sampler2D photo;
uniform sampler2D rings;
uniform float time;
in vec2 uv;
out vec4 color;
void main() {
  float r = texture(rings, uv).r;
  color = texture(photo, uv + 0.04 * vec2(sin(r * 6.28 + time), cos(r * 6.28 + time)));
}`;

const pixels = `#version 300 es
precision highp float;
uniform sampler2D tile;
in vec2 uv;
out vec4 color;
void main() {
  ivec2 size = textureSize(tile, 0);
  color = texelFetch(tile, ivec2(uv * vec2(size)), 0);
}`;

export default function Scene() {
  const { seconds } = useCurrentTime();
  const box = { width: 72, height: 72 };
  return (
    <View style={{ display: "flex", flexDirection: "row", gap: 12, padding: 12 }}>
      <Image src="resource://photo" style={{ ...box, objectFit: "fill" }} />
      <ShaderLayer
        code={ripple}
        inputs={{ photo: "resource://photo", rings: "resource://rings" }}
        uniforms={{ time: seconds * 6 }}
        style={box}
      />
      <ShaderLayer code={pixels} inputs={{ tile: "resource://tile" }} style={box} />
    </View>
  );
}
