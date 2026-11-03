// error: errors/shader-code.tsx:14:7: <ShaderLayer> code line 4: …
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

const code = `out vec4 color;
void main() {
  float x = 1.0;
  color = vec4(missing, x);
}`;

// A shader that names what it does not declare fails on its line.
export default function Scene() {
  return (
    <View>
      <ShaderLayer code={code} style={{ width: 8, height: 8 }} />
    </View>
  );
}
