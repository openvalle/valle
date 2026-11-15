export const composition = { width: 604, height: 340, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      key="canvas"
      style={{ position: "relative", width: "604px", height: "340px", backgroundColor: "white" }}
    >
      <View
        key="b0"
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
        }}
      >
        <View
          key="b0-0"
          style={{
            width: "38px",
            height: "auto",
            backgroundColor: "#2463a0",
            marginBlockEnd: "8px",
          }}
        ></View>
        <View
          key="b0-1"
          style={{
            width: "38px",
            height: "auto",
            backgroundColor: "#da803d",
            marginBlockEnd: "8px",
          }}
        ></View>
        <View
          key="b0-2"
          style={{
            width: "38px",
            height: "auto",
            backgroundColor: "#3b856c",
            marginBlockEnd: "8px",
          }}
        ></View>
      </View>
      <View
        key="b2"
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
        }}
      >
        <View
          key="b2-h"
          style={{ writingMode: "horizontal-tb", padding: "4px", backgroundColor: "#c6d6e6" }}
        >
          <View
            key="b2-a"
            style={{ width: "38px", height: "28px", backgroundColor: "#2463a0" }}
          ></View>
          <View
            key="b2-b"
            style={{ width: "58px", height: "28px", backgroundColor: "#da803d" }}
          ></View>
        </View>
        <View
          key="b2-v"
          style={{
            writingMode: "vertical-lr",
            width: "70px",
            height: "100px",
            marginBlockStart: "8px",
            backgroundColor: "#e1d0ec",
          }}
        >
          <View
            key="b2-c"
            style={{ width: "38px", height: "28px", backgroundColor: "#3b856c" }}
          ></View>
          <View
            key="b2-d"
            style={{ width: "38px", height: "28px", backgroundColor: "#936cb3" }}
          ></View>
        </View>
      </View>
      <View
        key="b1"
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
        }}
      >
        <View
          key="b1-0"
          style={{
            width: "38px",
            height: "auto",
            backgroundColor: "#2463a0",
            marginBlockEnd: "8px",
          }}
        ></View>
        <View
          key="b1-1"
          style={{
            width: "38px",
            height: "auto",
            backgroundColor: "#da803d",
            marginBlockEnd: "8px",
          }}
        ></View>
        <View
          key="b1-2"
          style={{
            width: "38px",
            height: "auto",
            backgroundColor: "#3b856c",
            marginBlockEnd: "8px",
          }}
        ></View>
      </View>
      <View
        key="b3"
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
        }}
      >
        <View
          key="b3-h"
          style={{ writingMode: "horizontal-tb", padding: "4px", backgroundColor: "#c6d6e6" }}
        >
          <View
            key="b3-a"
            style={{ width: "38px", height: "28px", backgroundColor: "#2463a0" }}
          ></View>
          <View
            key="b3-b"
            style={{ width: "58px", height: "28px", backgroundColor: "#da803d" }}
          ></View>
        </View>
        <View
          key="b3-v"
          style={{
            writingMode: "vertical-rl",
            width: "70px",
            height: "100px",
            marginBlockStart: "8px",
            backgroundColor: "#e1d0ec",
          }}
        >
          <View
            key="b3-c"
            style={{ width: "38px", height: "28px", backgroundColor: "#3b856c" }}
          ></View>
          <View
            key="b3-d"
            style={{ width: "38px", height: "28px", backgroundColor: "#936cb3" }}
          ></View>
        </View>
      </View>
    </View>
  );
}
