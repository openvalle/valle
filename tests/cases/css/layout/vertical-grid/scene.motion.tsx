export const composition = { width: 604, height: 340, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      key="canvas"
      style={{ position: "relative", width: "604px", height: "340px", backgroundColor: "white" }}
    >
      <View
        key="g0"
        style={{
          position: "absolute",
          left: "12px",
          top: "12px",
          width: "280px",
          height: "148px",
          padding: "10px",
          border: "2px solid #17365d",
          backgroundColor: "#eef3f8",
          writingMode: "vertical-rl",
          display: "grid",
          gridTemplateColumns: "36px 1fr",
          gridTemplateRows: "42px 1fr 36px",
          gap: "8px",
          direction: "ltr",
        }}
      >
        <View key="g0-0" style={{ backgroundColor: "#2463a0", gridColumn: "span 2" }}></View>
        <View key="g0-1" style={{ backgroundColor: "#da803d" }}></View>
        <View
          key="g0-2"
          style={{
            backgroundColor: "#3b856c",
            writingMode: "horizontal-tb",
            aspectRatio: "2",
            justifySelf: "start",
          }}
        ></View>
        <View key="g0-3" style={{ backgroundColor: "#936cb3" }}></View>
      </View>
      <View
        key="g1"
        style={{
          position: "absolute",
          left: "312px",
          top: "12px",
          width: "280px",
          height: "148px",
          padding: "10px",
          border: "2px solid #17365d",
          backgroundColor: "#eef3f8",
          writingMode: "vertical-lr",
          display: "grid",
          gridTemplateColumns: "36px 1fr",
          gridTemplateRows: "42px 1fr 36px",
          gap: "8px",
          direction: "ltr",
        }}
      >
        <View key="g1-0" style={{ backgroundColor: "#2463a0", gridColumn: "span 2" }}></View>
        <View key="g1-1" style={{ backgroundColor: "#da803d" }}></View>
        <View
          key="g1-2"
          style={{
            backgroundColor: "#3b856c",
            writingMode: "horizontal-tb",
            aspectRatio: "2",
            justifySelf: "start",
          }}
        ></View>
        <View key="g1-3" style={{ backgroundColor: "#936cb3" }}></View>
      </View>
      <View
        key="g2"
        style={{
          position: "absolute",
          left: "12px",
          top: "180px",
          width: "280px",
          height: "148px",
          padding: "10px",
          border: "2px solid #17365d",
          backgroundColor: "#eef3f8",
          writingMode: "vertical-rl",
          display: "grid",
          gridTemplateColumns: "36px 1fr",
          gridTemplateRows: "42px 1fr 36px",
          gap: "8px",
          direction: "rtl",
        }}
      >
        <View key="g2-0" style={{ backgroundColor: "#2463a0", gridColumn: "span 2" }}></View>
        <View key="g2-1" style={{ backgroundColor: "#da803d" }}></View>
        <View
          key="g2-2"
          style={{
            backgroundColor: "#3b856c",
            writingMode: "horizontal-tb",
            aspectRatio: "2",
            justifySelf: "start",
          }}
        ></View>
        <View key="g2-3" style={{ backgroundColor: "#936cb3" }}></View>
      </View>
      <View
        key="g3"
        style={{
          position: "absolute",
          left: "312px",
          top: "180px",
          width: "280px",
          height: "148px",
          padding: "10px",
          border: "2px solid #17365d",
          backgroundColor: "#eef3f8",
          writingMode: "vertical-lr",
          display: "grid",
          gridTemplateColumns: "36px 1fr",
          gridTemplateRows: "42px 1fr 36px",
          gap: "8px",
          direction: "rtl",
        }}
      >
        <View key="g3-0" style={{ backgroundColor: "#2463a0", gridColumn: "span 2" }}></View>
        <View key="g3-1" style={{ backgroundColor: "#da803d" }}></View>
        <View
          key="g3-2"
          style={{
            backgroundColor: "#3b856c",
            writingMode: "horizontal-tb",
            aspectRatio: "2",
            justifySelf: "start",
          }}
        ></View>
        <View key="g3-3" style={{ backgroundColor: "#936cb3" }}></View>
      </View>
    </View>
  );
}
