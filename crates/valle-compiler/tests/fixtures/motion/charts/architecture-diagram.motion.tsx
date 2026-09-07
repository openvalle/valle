// Architecture diagram fixture.
//

//
// graphLayout derives layers, ordering and alignment from node sizes and edges.
//
// Use public compute functions and visual primitives with resolution-independent coordinates.
export const component = "architecture-diagram";

// Input nodes and edges.
const NODES = [
  { label: "Client", tone: "#38bdf8" },
  { label: "Gateway", tone: "#a78bfa" },
  { label: "Auth", tone: "#a78bfa" },
  { label: "Router", tone: "#a78bfa" },
  { label: "Primary", tone: "#f472b6" },
  { label: "Replica", tone: "#f472b6" },
  { label: "Cache", tone: "#34d399" },
];
const EDGES = [
  [0, 1],
  [1, 2],
  [1, 3],
  [2, 4],
  [3, 4],
  [3, 5],
  [3, 6],
];

// Compute layout in a fixed canvas during preparation; apply viewport scaling at frame time.
const CANVAS_W = 1920;
const CANVAS_H = 1080;
const CARD_W = CANVAS_W * 0.115;
const CARD_H = CANVAS_H * 0.052;

// For top-down flow, cross is horizontal and flow is vertical.
const G = graphLayout({
  sizes: NODES.map(() => [CARD_W, CARD_H]),
  edges: EDGES,
  nodeGap: CANVAS_W * 0.028,
  rankGap: CANVAS_H * 0.055,
});

// Center the computed graph with one translation.
const SPAN_X = extent(G.centers.map((c) => c[0]));
const SPAN_Y = extent(G.centers.map((c) => c[1]));
const OFFSET_X = CANVAS_W / 2 - (SPAN_X[0] + SPAN_X[1]) / 2;
const OFFSET_Y = CANVAS_H / 2 - (SPAN_Y[0] + SPAN_Y[1]) / 2;
const CX = G.centers.map((c) => c[0] + OFFSET_X);
const CY = G.centers.map((c) => c[1] + OFFSET_Y);
const BEND = CANVAS_H * 0.055 * 0.6;
const LABEL_FONT = CANVAS_H * 0.024;

export default function ArchitectureDiagram(ctx) {
  const sx = ctx.viewport.width / CANVAS_W;
  const sy = ctx.viewport.height / CANVAS_H;

  // Stagger entrance by the computed layer index.
  const appear = (i) =>
    interpolate(ctx.enter.progress, [G.ranks[i] * 0.12, G.ranks[i] * 0.12 + 0.4], [0, 1]);

  return (
    <Scene className="h-full w-full bg-slate-950">
      {/* Draw edges behind cards and distinguish feedback edges. */}
      {EDGES.map((e, i) => (
        <Path
          key={`edge-${i}`}
          style={{ position: "absolute", left: 0, top: 0, opacity: appear(e[1]) }}
          fill="none"
          stroke={G.backEdges[i] ? "#f59e0b" : "#334155"}
          strokeWidth="4"
          strokeLinecap="round"
          d={cubic(
            point(CX[e[0]] * sx, (CY[e[0]] + CARD_H / 2) * sy),
            point(CX[e[0]] * sx, (CY[e[0]] + CARD_H / 2 + BEND) * sy),
            point(CX[e[1]] * sx, (CY[e[1]] - CARD_H / 2 - BEND) * sy),
            point(CX[e[1]] * sx, (CY[e[1]] - CARD_H / 2) * sy),
          )}
        />
      ))}

      {NODES.map((n, i) => (
        <View
          key={`card-${i}`}
          style={{
            borderStyle: "solid", position: "absolute",
            left: (CX[i] - CARD_W / 2) * sx,
            top: (CY[i] - CARD_H / 2) * sy,
            width: CARD_W * sx,
            height: CARD_H * sy,
            backgroundColor: "#0f172a",
            borderColor: n.tone,
            opacity: appear(i),
          }}
        />
      ))}
      {NODES.map((n, i) => (
        <Text
          key={`text-${i}`}
          style={{
            position: "absolute",
            left: (CX[i] - CARD_W / 2) * sx,
            top: (CY[i] - LABEL_FONT * 0.6) * sy,
            width: CARD_W * sx,
            fontSize: LABEL_FONT * sy,
            color: "#e2e8f0",
            textAlign: "center",
            opacity: appear(i),
          }}
        >
          {n.label}
        </Text>
      ))}
    </Scene>
  );
}
