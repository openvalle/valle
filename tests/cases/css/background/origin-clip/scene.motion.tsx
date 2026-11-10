export const composition = { width: 500, height: 170, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 500, height: 170, backgroundColor: "white" }}>
      <View
        key="border-box"
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 110,
          backgroundColor: "#eef2f7",
          padding: 8,
          border: "4px solid #102a43",
          backgroundImage: "linear-gradient(#176bba, #176bba)",
          backgroundSize: "40px 30px",
          backgroundRepeat: "no-repeat",
          backgroundPosition: "right bottom",
          backgroundOrigin: "border-box",
          backgroundClip: "content-box",
        }}
      />
      <View
        key="padding-box"
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 140,
          height: 110,
          backgroundColor: "#eef2f7",
          padding: 8,
          border: "4px solid #102a43",
          backgroundImage: "linear-gradient(#176bba, #176bba)",
          backgroundSize: "40px 30px",
          backgroundRepeat: "no-repeat",
          backgroundPosition: "right bottom",
          backgroundOrigin: "padding-box",
          backgroundClip: "content-box",
        }}
      />
      <View
        key="content-box"
        style={{
          position: "absolute",
          left: 340,
          top: 20,
          width: 140,
          height: 110,
          backgroundColor: "#eef2f7",
          padding: 8,
          border: "4px solid #102a43",
          backgroundImage: "linear-gradient(#176bba, #176bba)",
          backgroundSize: "40px 30px",
          backgroundRepeat: "no-repeat",
          backgroundPosition: "right bottom",
          backgroundOrigin: "content-box",
          backgroundClip: "content-box",
        }}
      />
    </View>
  );
}
