export const composition = { width: 360, height: 220, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 360, height: 220, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 80,
          backgroundColor: "white",
          backgroundImage: "linear-gradient(90deg in srgb, #000000, #ffffff)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 200,
          top: 20,
          width: 140,
          height: 80,
          backgroundColor: "white",
          backgroundImage: "linear-gradient(to right, #176bba 50%, #ffb74d 50%)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 130,
          width: 140,
          height: 70,
          backgroundColor: "white",
          backgroundImage:
            "repeating-linear-gradient(to right, #176bba 0px 10px, #ffb74d 10px 20px)",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 200,
          top: 130,
          width: 140,
          height: 70,
          backgroundColor: "white",
          backgroundImage: "radial-gradient(circle at center, white 0px, #176bba 30px)",
        }}
      />
    </View>
  );
}
