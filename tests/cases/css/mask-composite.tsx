export const composition = { width: 256, height: 170, fps: 1, duration: 1 };

// mask-composite: two layers of soft gradients added, subtracted, intersected and excluded; three
// layers, each going with those under it, the bottom one's composite of no use; a layer by
// luminance in the middle; one of no image, which leaves nothing where it intersects; and
// mask-clip and mask-origin, on a box with a border and padding.
export default function Scene() {
  const fill = "linear-gradient(135deg, #ffca28, #e57373 50%, #00897b)";
  const two = "linear-gradient(to right, #000, transparent), linear-gradient(#000, transparent)";
  const box = (style: object) => (
    <View style={{ width: 54, height: 54, background: fill, ...style }} />
  );
  const spaced = {
    border: "6px solid #3949ab",
    padding: 6,
    maskImage: "radial-gradient(circle, #000 60%, transparent 62%)",
    maskRepeat: "no-repeat",
  };
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: 6, background: "#eceff1" }}>
      {box({ maskImage: two, maskComposite: "add" })}
      {box({ maskImage: two, maskComposite: "subtract" })}
      {box({ maskImage: two, maskComposite: "intersect" })}
      {box({ maskImage: two, maskComposite: "exclude" })}
      {box({
        maskImage: `radial-gradient(circle, #000 50%, transparent 70%), ${two}`,
        maskComposite: "intersect, add, subtract",
      })}
      {box({
        maskImage: `linear-gradient(45deg, #000, transparent), conic-gradient(#fff, #000, #fff), linear-gradient(#000, #000)`,
        maskMode: "alpha, luminance, alpha",
        maskComposite: "exclude, intersect",
      })}
      {box({ maskImage: "linear-gradient(#000, #000), none", maskComposite: "intersect" })}
      {box({ ...spaced, maskClip: "padding-box" })}
      {box({ ...spaced, maskClip: "content-box" })}
      {box({ ...spaced, maskOrigin: "padding-box", maskSize: "50% 50%", maskClip: "padding-box" })}
      {box({ ...spaced, maskOrigin: "content-box", maskSize: "50% 50%" })}
    </View>
  );
}
