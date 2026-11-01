// error: errors/inline-block.tsx:7:7: display: inline-block needs a flex parent: there is no HTML inline flow; use flex layout, and Span for styled runs of text
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View>
      <View style={{ display: "inline-block" }} />
    </View>
  );
}
