// error: errors/role-ends.tsx: role: the intro and the outro, 2.5 s, are longer than the composition, 2 s
export const composition = { width: 64, height: 64, fps: 1, duration: 2 };

// An overlay keeps its intro and outro whole, so the scene holds them.
export const role = overlay({ intro: 1.5, outro: 1, hold: "stretch" });

export default function Scene() {
  return <View style={{ width: 8, height: 8 }} />;
}
