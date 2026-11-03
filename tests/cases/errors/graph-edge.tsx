// error: errors/graph-edge.tsx:5: RangeError: graphLayout's edge [0, 3] names 3, and the nodes are 0 to 2…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  const g = graphLayout({
    sizes: [
      [10, 10],
      [10, 10],
      [10, 10],
    ],
    edges: [[0, 3]],
  });
  return <View style={{ width: g.centers[0][0] }} />;
}
