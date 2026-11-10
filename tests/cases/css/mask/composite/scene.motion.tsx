export const composition = { width: 660, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ position: "relative", width: 660, height: 180, backgroundColor: "white" }}>
      <View
        style={{
          position: "absolute",
          left: 20,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#176bba",
          maskImage:
            "linear-gradient(to right, black 65%, transparent 65%), linear-gradient(to bottom, black 50%, transparent 50%)",
          maskComposite: "add",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 100,
            height: 100,
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 180,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#176bba",
          maskImage:
            "linear-gradient(to right, black 65%, transparent 65%), linear-gradient(to bottom, black 50%, transparent 50%)",
          maskComposite: "subtract",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 100,
            height: 100,
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 340,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#176bba",
          maskImage:
            "linear-gradient(to right, black 65%, transparent 65%), linear-gradient(to bottom, black 50%, transparent 50%)",
          maskComposite: "intersect",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 100,
            height: 100,
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
      <View
        style={{
          position: "absolute",
          left: 500,
          top: 20,
          width: 140,
          height: 140,
          backgroundColor: "#176bba",
          maskImage:
            "linear-gradient(to right, black 65%, transparent 65%), linear-gradient(to bottom, black 50%, transparent 50%)",
          maskComposite: "exclude",
        }}
      >
        <View
          style={{
            position: "absolute",
            left: 40,
            top: 40,
            width: 100,
            height: 100,
            backgroundColor: "#ffb74d",
          }}
        />
      </View>
    </View>
  );
}
