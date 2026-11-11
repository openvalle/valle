// The reel's last card: Valle's three bars spring in, then the line and the command that
// renders the reel. Designed at 1920 x 1080, 30 fps, 3 s.

export const composition = { width: 1920, height: 1080, fps: 30, duration: 3 };

// The fonts are Google Fonts' files through jsDelivr, at a pinned commit, under the SIL Open Font License 1.1.
export const controls = {
  resources: {
    Geist: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geist/Geist%5Bwght%5D.ttf",
    }),
    GeistMono: resource({
      kind: "font",
      default:
        "https://cdn.jsdelivr.net/gh/google/fonts@5bb28733cec313b63fbd415b574b5ae35c8aa8b8/ofl/geistmono/GeistMono%5Bwght%5D.ttf",
    }),
  },
};

const NIGHT = "#0A0B0D";
const WHITE = "#F2F3F5";
const COMMAND = "valle timeline render reel.timeline.json -o reel.mp4";

// The mark, as web/apps/studio/brand/valle-mark.svg draws it, three times the size: 348 x 300.
const BARS = [
  { key: "red", y: 0, width: 228, color: "#F5534B" },
  { key: "green", y: 108, width: 348, color: "#24BA68" },
  { key: "blue", y: 216, width: 228, color: "#347FF0" },
];

export default function ReelCard() {
  const { seconds } = useCurrentTime();
  const line = interpolate(seconds, [0.75, 1.45], [0, 1], { easing: "easeOutCubic" });
  const prompt = interpolate(seconds, [1.2, 1.6], [0, 1], { easing: "easeOutCubic" });
  // The command is typed, a character every 20 ms.
  const typed = clamp((seconds - 1.45) / 0.02, 0, COMMAND.length);
  return (
    <View
      className="relative flex h-full w-full flex-col items-center justify-center"
      style={{ backgroundColor: NIGHT }}
    >
      <View
        key="glow"
        className="absolute inset-0"
        style={{
          backgroundImage:
            "radial-gradient(ellipse 50% 45% at 50% 42%, #1E2330 0%, #0A0B0D00 100%)",
        }}
      />
      <View key="mark" className="relative" style={{ width: 348, height: 300 }}>
        {BARS.map((bar, index) => {
          const grow = spring(seconds - 0.2 - index * 0.06, { duration: 0.6, bounce: 0.3 });
          return (
            <View
              key={bar.key}
              className="absolute"
              style={{
                left: 0,
                top: bar.y,
                width: bar.width,
                height: 84,
                borderRadius: 42,
                backgroundColor: bar.color,
                scale: `${grow} 1`,
                transformOrigin: "0% 50%",
                opacity: clamp(grow * 4, 0, 1),
              }}
            />
          );
        })}
      </View>
      <Text
        key="line"
        style={{
          marginTop: 80,
          fontFamily: "Geist",
          fontSize: 104,
          fontWeight: 500,
          letterSpacing: -2.5,
          color: WHITE,
          opacity: line,
          translate: `0px ${(1 - line) * 20}px`,
        }}
      >
        The video engine for AI agents.
      </Text>
      <View
        key="command"
        className="flex items-center"
        style={{
          marginTop: 64,
          height: 104,
          paddingLeft: 40,
          paddingRight: 40,
          gap: 24,
          borderRadius: 20,
          backgroundColor: "#16181D",
          border: "1.5px solid #2A2E37",
          opacity: prompt,
        }}
      >
        <Text style={{ fontFamily: "GeistMono", fontSize: 40, color: "#24BA68" }}>$</Text>
        <Text
          split="char"
          style={{ fontFamily: "GeistMono", fontSize: 40, color: "#C9CDD6" }}
          perUnit={(unit) => ({ opacity: clamp(typed - unit.index, 0, 1) })}
        >
          {COMMAND}
        </Text>
      </View>
    </View>
  );
}
