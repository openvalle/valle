export const composition = { width: 340, height: 160, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View
      style={{
        width: 340,
        height: 160,
        backgroundColor: "white",
        display: "flex",
        gap: 20,
        padding: 10,
      }}
    >
      <View
        style={{
          width: 150,
          height: 140,
          fontSize: 20,
          padding: "0.5em",
          border: "4px solid #1264a3",
          backgroundColor: "#e8edf2",
        }}
      >
        <View
          key="inherit"
          style={{
            width: 100,
            height: 70,
            fontSize: 10,
            padding: "inherit",
            paddingLeft: 4,
            border: "inherit",
            borderColor: "initial",
            color: "#a63a30",
            backgroundColor: "inherit",
          }}
        >
          <View style={{ width: 20, height: 20, backgroundColor: "#f4b642" }} />
        </View>
      </View>
      <View
        style={{
          width: 150,
          height: 140,
          padding: 10,
          border: "4px solid #1264a3",
          backgroundColor: "#e8edf2",
        }}
      >
        <View
          key="reset"
          style={{
            width: 100,
            height: 70,
            padding: 20,
            border: "inherit",
            paddingTop: "unset",
            borderWidth: "initial",
            borderColor: "#a63a30",
            backgroundColor: "#f4b642",
            opacity: "unset",
          }}
        />
      </View>
    </View>
  );
}
