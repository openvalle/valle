export const composition = { width: 256, height: 160, fps: 1, duration: 1 };

// Color functions as swatches: oklch and oklab, the first out of sRGB, which clips as Chrome
// clips it; lab and lch, in numbers and percentages; hwb, and a gray of whiteness and
// blackness; color() in each predefined space; a missing channel; and translucent ones.
const colors = [
  "oklch(0.7 0.1 200)",
  "oklch(0.7 0.4 30)",
  "oklch(0.9 0.3 140)",
  "oklab(0.5 0.1 -0.1)",
  "oklab(50% 25% -25%)",
  "lab(50 40 -20)",
  "lab(50% 32% -16%)",
  "lch(60 50 120)",
  "lch(50 200 300)",
  "hwb(200 20% 30%)",
  "hwb(0 60% 60%)",
  "color(display-p3 1 0.5 0)",
  "color(srgb 0.2 0.4 0.6)",
  "color(srgb-linear 0.2 0.4 0.6)",
  "color(rec2020 0.5 0.3 0.1)",
  "color(a98-rgb 0.5 0.3 0.1)",
  "color(prophoto-rgb 0.5 0.3 0.1)",
  "color(xyz 0.2 0.3 0.4)",
  "color(xyz-d50 0.2 0.3 0.4)",
  "oklch(0.7 none 200)",
  "oklch(0.5 0.1 200 / 50%)",
  "lab(60 -40 40 / 0.25)",
  "color(display-p3 0 0 1 / 75%)",
  "hwb(120 10% 10% / 0.6)",
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
