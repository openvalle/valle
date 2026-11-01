export const composition = { width: 500, height: 140, fps: 1, duration: 1 };

export const controls = {
  resources: {
    icon: resource({ kind: "image", default: "../resources/icon.svg" }),
    logo: resource({ kind: "image", default: "../resources/logo.svg" }),
    ruler: resource({ kind: "image", default: "../resources/ruler.svg" }),
  },
};

// SVG files in Images: an icon with a viewBox and no size, its currentColor black whatever the
// Image's color, as its width makes it, contained and covered in a rounded box; a logo an editor
// saved, its gradient, clip path, copies and markers by ids an inline <svg> beside it uses too, at
// its natural size, half of it, in a square, which its viewBox fits it in, and unscaled at the top
// right; and a file sized in mm without a viewBox, at its natural size and in a wider box, which
// stretches it as it would an image.
export default function Scene() {
  const white = { background: "#ffffff" };
  return (
    <View
      style={{
        display: "flex",
        flexWrap: "wrap",
        gap: 6,
        padding: 6,
        alignItems: "flex-start",
        background: "#eceff1",
      }}
    >
      <Image src="resource://icon" style={{ width: 48, color: "#e53935" }} />
      <Image
        src="resource://icon"
        style={{ width: 80, height: 40, objectFit: "contain", ...white }}
      />
      <Image
        src="resource://icon"
        style={{ width: 80, height: 40, objectFit: "cover", borderRadius: 12 }}
      />
      <Image src="resource://logo" />
      <Image src="resource://logo" style={{ width: 60 }} />
      <Image src="resource://logo" style={{ width: 60, height: 60 }} />
      <Image
        src="resource://logo"
        style={{ width: 100, height: 60, objectFit: "none", objectPosition: "right top", ...white }}
      />
      <Image src="resource://ruler" />
      <Image src="resource://ruler" style={{ width: 160 }} />
      <svg width={40} height={40}>
        <linearGradient id="g">
          <stop offset={0} stopColor="#00897b" />
          <stop offset={1} stopColor="#c0ca33" />
        </linearGradient>
        <clipPath id="c">
          <rect x={4} y={4} width={32} height={32} rx={8} />
        </clipPath>
        <rect width={40} height={40} fill="url(#g)" clipPath="url(#c)" />
      </svg>
    </View>
  );
}
