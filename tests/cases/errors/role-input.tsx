// error: errors/role-input.tsx:6:10: <Image> src input://from: the Timeline gives it to a scene that declares `export const role = transition()`
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// A scene reads the Timeline's pictures only in the role that gets them.
export default function Scene() {
  return <Image src="input://from" style={{ width: 8, height: 8 }} />;
}
