export const composition = { width: 720, height: 300, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 720, height: 300, backgroundColor: "white" }}>
      <Text
        key="row0"
        style={{
          position: "absolute",
          left: 12,
          top: 12,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a0"
          style={{
            verticalAlign: "baseline",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row1"
        style={{
          position: "absolute",
          left: 252,
          top: 12,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a1"
          style={{
            verticalAlign: "middle",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row2"
        style={{
          position: "absolute",
          left: 492,
          top: 12,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a2"
          style={{
            verticalAlign: "sub",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row3"
        style={{
          position: "absolute",
          left: 12,
          top: 82,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a3"
          style={{
            verticalAlign: "super",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row4"
        style={{
          position: "absolute",
          left: 252,
          top: 82,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a4"
          style={{
            verticalAlign: "text-top",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row5"
        style={{
          position: "absolute",
          left: 492,
          top: 82,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a5"
          style={{
            verticalAlign: "text-bottom",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row6"
        style={{
          position: "absolute",
          left: 12,
          top: 152,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a6"
          style={{
            verticalAlign: "top",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row7"
        style={{
          position: "absolute",
          left: 252,
          top: 152,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a7"
          style={{
            verticalAlign: "bottom",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row8"
        style={{
          position: "absolute",
          left: 492,
          top: 152,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a8"
          style={{
            verticalAlign: "8px",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row9"
        style={{
          position: "absolute",
          left: 12,
          top: 222,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a9"
          style={{
            verticalAlign: "-8px",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row10"
        style={{
          position: "absolute",
          left: 252,
          top: 222,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          key="a10"
          style={{
            verticalAlign: "25%",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          xy
        </Span>
        B
      </Text>
      <Text
        key="row11"
        style={{
          position: "absolute",
          left: 492,
          top: 222,
          width: 220,
          height: 58,
          fontSize: 32,
          lineHeight: "48px",
          color: "#17365d",
        }}
      >
        A
        <Span
          style={{
            verticalAlign: "top",
            fontSize: 20,
            lineHeight: "26px",
            backgroundColor: "#ffebae",
          }}
        >
          x
          <Span
            style={{
              verticalAlign: "super",
              fontSize: 12,
              lineHeight: "16px",
              backgroundColor: "#d6eaf8",
            }}
          >
            2
          </Span>
        </Span>
        B
      </Text>
    </View>
  );
}
