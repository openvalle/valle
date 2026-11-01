export const composition = { width: 256, height: 128, fps: 1, duration: 1 };

// Colors mixed: in Oklab when no space is given; in polar spaces, where a gray's hue is
// powerless and a missing hue comes from the other color; each way round the hue; with
// percentages that add up to less than 100%, which leaves the rest transparent; with
// transparent; a color past sRGB, mixed before it clips; and a mix of a mix.
const colors = [
  "color-mix(red, blue)",
  "color-mix(in srgb, red 30%, blue)",
  "color-mix(in srgb-linear, red, blue)",
  "color-mix(in oklch, white, blue)",
  "color-mix(in hsl, white, blue)",
  "color-mix(in lch, gray, red)",
  "color-mix(in oklch, oklch(0.6 0.2 none), oklch(0.6 0.2 120))",
  "color-mix(in oklch longer hue, red, blue)",
  "color-mix(in oklch increasing hue, oklch(0.6 0.15 10), oklch(0.6 0.15 350))",
  "color-mix(in srgb, red 30%, blue 30%)",
  "color-mix(in oklch, red 25%, transparent)",
  "color-mix(in lab, red, rgb(0 0 255 / 50%))",
  "color-mix(in srgb, oklch(0.7 0.4 30), white)",
  "color-mix(in display-p3, red, blue)",
  "color-mix(in hwb, red, blue)",
  "color-mix(in oklab, color-mix(in srgb, red, blue), white)",
];

export default function Scene() {
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 4, padding: 4, background: "#808080" }}>
      {colors.map((c) => (
        <View style={{ width: 28, height: 28, background: c }} />
      ))}
    </View>
  );
}
