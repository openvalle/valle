export const composition = { width: 360, height: 220, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 220, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 70,
          height: 70,
          backgroundColor: "white",
          border: "6px solid #176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 105,
          top: 20,
          width: 70,
          height: 70,
          backgroundColor: "white",
          border: "6px double #176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 190,
          top: 20,
          width: 70,
          height: 70,
          backgroundColor: "white",
          border: "6px dashed #176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 275,
          top: 20,
          width: 70,
          height: 70,
          backgroundColor: "white",
          border: "6px dotted #176bba",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 130,
          width: 120,
          height: 60,
          backgroundColor: "#ffb74d",
          border: "4px solid #102a43",
          borderRadius: "20px 8px",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 200,
          top: 120,
          width: 120,
          height: 80,
          backgroundColor: "#16a085",
          borderRadius: "50%",
        }}
      />
    </View>
  );
}
