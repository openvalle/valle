export const composition = { width: 604, height: 340, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      key="canvas"
      style={{ position: "relative", width: "604px", height: "340px", backgroundColor: "white" }}
    >
      <View
        key="p0"
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
          direction: "ltr",
        }}
      >
        <View
          key="p0-a"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#2463a0",
            position: "absolute",
            insetInlineStart: "14px",
            insetBlockStart: "18px",
          }}
        ></View>
        <View
          key="p0-b"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#da803d",
            position: "absolute",
            right: "8px",
            bottom: "12px",
            padding: "6.25%",
          }}
        ></View>
        <View
          key="p0-nested"
          style={{
            writingMode: "horizontal-tb",
            width: "80px",
            height: "60px",
            padding: "6px",
            backgroundColor: "#c6d6e6",
          }}
        >
          <View
            key="p0-c"
            style={{
              width: "38px",
              height: "28px",
              backgroundColor: "#3b856c",
              position: "absolute",
            }}
          ></View>
        </View>
      </View>
      <View
        key="p1"
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
          direction: "ltr",
        }}
      >
        <View
          key="p1-a"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#2463a0",
            position: "absolute",
            insetInlineStart: "14px",
            insetBlockStart: "18px",
          }}
        ></View>
        <View
          key="p1-b"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#da803d",
            position: "absolute",
            right: "8px",
            bottom: "12px",
            padding: "6.25%",
          }}
        ></View>
        <View
          key="p1-nested"
          style={{
            writingMode: "horizontal-tb",
            width: "80px",
            height: "60px",
            padding: "6px",
            backgroundColor: "#c6d6e6",
          }}
        >
          <View
            key="p1-c"
            style={{
              width: "38px",
              height: "28px",
              backgroundColor: "#3b856c",
              position: "absolute",
            }}
          ></View>
        </View>
      </View>
      <View
        key="p2"
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
          direction: "rtl",
        }}
      >
        <View
          key="p2-a"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#2463a0",
            position: "absolute",
            insetInlineStart: "14px",
            insetBlockStart: "18px",
          }}
        ></View>
        <View
          key="p2-b"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#da803d",
            position: "absolute",
            right: "8px",
            bottom: "12px",
            padding: "6.25%",
          }}
        ></View>
        <View
          key="p2-nested"
          style={{
            writingMode: "horizontal-tb",
            width: "80px",
            height: "60px",
            padding: "6px",
            backgroundColor: "#c6d6e6",
          }}
        >
          <View
            key="p2-c"
            style={{
              width: "38px",
              height: "28px",
              backgroundColor: "#3b856c",
              position: "absolute",
            }}
          ></View>
        </View>
      </View>
      <View
        key="p3"
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
          direction: "rtl",
        }}
      >
        <View
          key="p3-a"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#2463a0",
            position: "absolute",
            insetInlineStart: "14px",
            insetBlockStart: "18px",
          }}
        ></View>
        <View
          key="p3-b"
          style={{
            width: "38px",
            height: "28px",
            backgroundColor: "#da803d",
            position: "absolute",
            right: "8px",
            bottom: "12px",
            padding: "6.25%",
          }}
        ></View>
        <View
          key="p3-nested"
          style={{
            writingMode: "horizontal-tb",
            width: "80px",
            height: "60px",
            padding: "6px",
            backgroundColor: "#c6d6e6",
          }}
        >
          <View
            key="p3-c"
            style={{
              width: "38px",
              height: "28px",
              backgroundColor: "#3b856c",
              position: "absolute",
            }}
          ></View>
        </View>
      </View>
    </View>
  );
}
