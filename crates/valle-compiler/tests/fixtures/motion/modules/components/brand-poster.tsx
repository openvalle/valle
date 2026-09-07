import { releaseTheme } from "./shared-theme";

export const component = "brand-poster";

export const controls = defineControls({
  props: { accentStrength: number({ default: 1, min: 0.25, max: 1 }) },
});

const COUNT = 10000;
const GRID = Array.from({ length: COUNT }, (_, index) => {
  const column = index % 100;
  const row = Math.floor(index / 100);
  return point(26 + column * 18.8, 18 + row * 10.5);
});
const SIGNAL = Array.from({ length: COUNT }, (_, index) => {
  const column = index % 100;
  const row = Math.floor(index / 100);
  return point(120 + row * 16.8 + (column % 10) * 4.2, 40 + column * 9.8 + (row % 8) * 5.4);
});

export default function BrandPoster(ctx, props) {
  const progress = ctx.hold.progress;
  const viewportScale = Math.min(ctx.viewport.width / 1920, ctx.viewport.height / 1080);
  const metric = 72 + progress * 27.9;
  return (
    <ThemeProvider value={releaseTheme}>
      <Scene key="scene" className="relative h-full w-full bg-linear-to-r from-cyan-400 via-blue-500 to-violet-500" camera={{ center: point(960, 540), zoom: viewportScale, rotation: 0 }}>
        <World key="world">
          <View key="wash" className="absolute" style={{ left: 0, top: 0, width: 1920, height: 1080, background: "linear-gradient(135deg, #020617 0%, #111d4a 52%, #4a154b 100%)" }} />
          <GeometryBatch key="dense-brand-field" geometry="circle"
            positions={field({ from: GRID, to: SIGNAL, progress, stagger: 0.00005 })}
            sizes={field({ from: 0.7, to: 2.4, progress })}
            fills={field({ from: "#22d3ee", to: "#f472b6", progress })}
            opacities={field({ from: 0.08, to: 0.54, progress, stagger: 0.00004 })} />
          <View key="glass" className="absolute" style={{ borderStyle: "solid", left: 84, top: 82, width: 1752, height: 916, borderRadius: 44, backgroundColor: "#020617ad", borderWidth: 1, borderColor: "#ffffff35", backdropFilter: "blur(18px)", overflow: "hidden" }}>
            <View key="grid" className="absolute" style={{ left: 0, top: 0, width: 1752, height: 916, backgroundImage: "repeating-linear-gradient(0deg, transparent 0 79px, #ffffff12 79px 80px), repeating-linear-gradient(90deg, transparent 0 79px, #ffffff12 79px 80px)", opacity: props.accentStrength }} />
            <Text key="issue" className="absolute" style={{ left: 54, top: 46, fontSize: 16, letterSpacing: 4, color: useTheme().colors.accent }}>VALLE FIELD NOTES / 2026</Text>
            <Text key="headline" className="absolute whitespace-pre" split="char" perUnit={{ opacity: interpolate(progress - ctx.unit.index * 0.012, [0, 0.24], [0, 1]), translate: point(0, interpolate(progress - ctx.unit.index * 0.012, [0, 0.3], [48, 0], { easing: "easeOut" })) }} style={{ left: 48, top: 128, width: 1080, fontSize: 164, lineHeight: 0.88, letterSpacing: -7, color: useTheme().colors.ink }}>{"FRAME\nPURE"}</Text>
            <Text key="manifesto" className="absolute" style={{ left: 62, top: 520, width: 860, fontSize: 30, lineHeight: 1.35, color: useTheme().colors.muted }}>Ten thousand marks. One command. Every parameter remains editable by a human or an agent.</Text>
            <View key="metric-card" className="absolute" style={{ borderStyle: "solid", right: 54, top: 84, width: 510, height: 430, borderRadius: 30, backgroundColor: "#0f172acc", borderWidth: 1, borderColor: useTheme().colors.hairline, filter: "drop-shadow(0px 28px 70px #00000088)" }}>
              <Text key="metric-label" className="absolute" style={{ left: 36, top: 34, fontSize: 16, letterSpacing: 3, color: useTheme().colors.signal }}>DETERMINISM</Text>
              <Text key="metric-value" className="absolute tabular-nums" style={{ left: 30, top: 88, width: 450, fontSize: 126, letterSpacing: -5, color: useTheme().colors.ink, textAlign: "center" }}>{formatNumber(metric, { decimals: 1 })}</Text>
              <Text key="metric-unit" className="absolute" style={{ left: 36, top: 260, width: 438, fontSize: 24, color: useTheme().colors.accent, textAlign: "center" }}>PERCENT / SEEK STABLE</Text>
              <View key="metric-track" className="absolute" style={{ left: 36, bottom: 52, width: 438, height: 8, borderRadius: 5, backgroundColor: "#ffffff1c" }} />
              <View key="metric-fill" className="absolute" style={{ left: 36, bottom: 52, width: 438 * progress, height: 8, borderRadius: 5, backgroundColor: useTheme().colors.signal }} />
            </View>
            <Text key="footer" className="absolute" style={{ left: 62, bottom: 48, fontSize: 17, letterSpacing: 2.6, color: useTheme().colors.accent }}>MOTION JSX · TAKUMI · DISPLAYLIST</Text>
          </View>
        </World>
      </Scene>
    </ThemeProvider>
  );
}
