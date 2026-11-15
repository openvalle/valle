export const composition = { width: 604, height: 340, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      key="canvas"
      style={{ position: "relative", width: "604px", height: "340px", backgroundColor: "white" }}
    >
      <View
        key="f0"
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
          display: "flex",
          flexDirection: "row",
          flexWrap: "wrap",
          gap: "8px",
          alignContent: "space-between",
          justifyContent: "center",
          direction: "ltr",
        }}
      >
        <View
          key="f0-0"
          style={{ width: "70px", height: "42px", backgroundColor: "#2463a0", flexShrink: "0" }}
        ></View>
        <View
          key="f0-1"
          style={{ width: "70px", height: "42px", backgroundColor: "#da803d", flexShrink: "0" }}
        ></View>
        <View
          key="f0-2"
          style={{ width: "70px", height: "42px", backgroundColor: "#3b856c", flexShrink: "0" }}
        ></View>
        <View
          key="f0-3"
          style={{ width: "70px", height: "42px", backgroundColor: "#936cb3", flexShrink: "0" }}
        ></View>
      </View>
      <View
        key="f1"
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
          display: "flex",
          flexDirection: "row-reverse",
          flexWrap: "wrap",
          gap: "8px",
          alignContent: "space-between",
          justifyContent: "center",
          direction: "rtl",
        }}
      >
        <View
          key="f1-0"
          style={{ width: "70px", height: "42px", backgroundColor: "#2463a0", flexShrink: "0" }}
        ></View>
        <View
          key="f1-1"
          style={{ width: "70px", height: "42px", backgroundColor: "#da803d", flexShrink: "0" }}
        ></View>
        <View
          key="f1-2"
          style={{ width: "70px", height: "42px", backgroundColor: "#3b856c", flexShrink: "0" }}
        ></View>
        <View
          key="f1-3"
          style={{ width: "70px", height: "42px", backgroundColor: "#936cb3", flexShrink: "0" }}
        ></View>
      </View>
      <View
        key="f2"
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
          display: "flex",
          flexDirection: "column",
          flexWrap: "wrap",
          gap: "8px",
          alignContent: "space-between",
          justifyContent: "center",
          direction: "ltr",
        }}
      >
        <View
          key="f2-0"
          style={{ width: "70px", height: "42px", backgroundColor: "#2463a0", flexShrink: "0" }}
        ></View>
        <View
          key="f2-1"
          style={{ width: "70px", height: "42px", backgroundColor: "#da803d", flexShrink: "0" }}
        ></View>
        <View
          key="f2-2"
          style={{ width: "70px", height: "42px", backgroundColor: "#3b856c", flexShrink: "0" }}
        ></View>
        <View
          key="f2-3"
          style={{ width: "70px", height: "42px", backgroundColor: "#936cb3", flexShrink: "0" }}
        ></View>
      </View>
      <View
        key="f3"
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
          display: "flex",
          flexDirection: "column-reverse",
          flexWrap: "wrap",
          gap: "8px",
          alignContent: "space-between",
          justifyContent: "center",
          direction: "ltr",
        }}
      >
        <View
          key="f3-0"
          style={{ width: "70px", height: "42px", backgroundColor: "#2463a0", flexShrink: "0" }}
        ></View>
        <View
          key="f3-1"
          style={{ width: "70px", height: "42px", backgroundColor: "#da803d", flexShrink: "0" }}
        ></View>
        <View
          key="f3-2"
          style={{ width: "70px", height: "42px", backgroundColor: "#3b856c", flexShrink: "0" }}
        ></View>
        <View
          key="f3-3"
          style={{ width: "70px", height: "42px", backgroundColor: "#936cb3", flexShrink: "0" }}
        ></View>
      </View>
    </View>
  );
}
