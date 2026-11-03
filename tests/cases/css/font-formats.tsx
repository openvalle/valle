export const controls = {
  resources: {
    otf: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/OTF/SourceSans3-Regular.otf",
    }),
    ttf: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/TTF/SourceSans3-Regular.ttf",
    }),
    cff2: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/VF/SourceSans3VF-Upright.otf",
    }),
    woff: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/WOFF/TTF/SourceSans3-Regular.ttf.woff",
    }),
    woff2: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/WOFF2/OTF/SourceSans3-Regular.otf.woff2",
    }),
    woff2ttf: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/WOFF2/TTF/SourceSans3-Regular.ttf.woff2",
    }),
    woff2vf: resource({
      kind: "font",
      default:
        "https://raw.githubusercontent.com/adobe-fonts/source-sans/87b37a2daaed80fcb8e8ccb0085c4d72ddade12e/WOFF2/VF/SourceSans3VF-Upright.otf.woff2",
    }),
  },
};

export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Source Sans 3 in each format: CFF and CFF2 outlines, cubic, as quadratic curves; WOFF and
// WOFF2 decoded, TrueType glyphs transformed in WOFF2; the variable ones also at their heaviest
// master (Chrome makes a bold of a static font, which the engine does not). Variable TrueType
// is not here: at 400, between its masters, its advances are a known difference.
const FORMATS = [
  ["otf", "OpenType CFF", false],
  ["ttf", "TrueType", false],
  ["cff2", "CFF2", true],
  ["woff", "WOFF", false],
  ["woff2", "WOFF2 CFF", false],
  ["woff2ttf", "WOFF2 TrueType", false],
  ["woff2vf", "WOFF2 CFF2", true],
] as const;

export default function Scene() {
  return (
    <View style={{ padding: 6, gap: 4, color: "#263238" }}>
      {FORMATS.map(([family, label, variable]) => (
        <Text key={family} style={{ fontFamily: family, fontSize: 22 }}>
          {label}
          {variable && <Span style={{ fontWeight: 900, color: "#e53935" }}> Black 900</Span>}
        </Text>
      ))}
    </View>
  );
}
