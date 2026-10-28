export const composition = { width: 320, height: 180, fps: 30, duration: 3 };

// At the registered 320x180 QA size, the tracked group moves exactly one pixel/frame.
// It combines 1px lines, a 0.5px hairline, and partial alpha. The rest exercises small
// text, high-frequency stripes, dark gradients and the admitted CPU blur range.
export default function QualityDetail() {
  const time = useCurrentTime();
  const frame = Math.round(time.seconds * 30);
  return <Scene style={{ width: 320, height: 180 }}>
    <View style={{ position: "absolute", left: 8 + frame, top: 16, width: 64, height: 52 }}>
      <View style={{ position: "absolute", left: 2, top: 2, width: 48, height: 32, backgroundColor: "#5ab8e080" }} />
      <View style={{ position: "absolute", left: 4, top: 6, width: 1, height: 36, backgroundColor: "#fff" }} />
      <View style={{ position: "absolute", left: 4, top: 40, width: 48, height: 1, backgroundColor: "#fff" }} />
      <Path style={{ position: "absolute", left: 0, top: 0, width: 64, height: 52 }} d="M 8 8 L 48 36" fill="none" stroke="#ffc080" strokeWidth={0.5} />
    </View>
    <View style={{ position: "absolute", left: 240, top: 24, width: 16, height: 16, backgroundColor: "#d060c0", opacity: 0.5, filter: "blur(16px)" }} />
    <Text style={{ position: "absolute", left: 12, top: 84, fontSize: 8, color: "#c0c0c0" }}>VALLE 012345 thin details</Text>
    <View style={{ position: "absolute", left: 12, top: 108, width: 288, height: 24, backgroundImage: "linear-gradient(90deg, #000000, #202020)" }} />
    <View style={{ position: "absolute", left: 12, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 13, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 14, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 15, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 16, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 17, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 18, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 19, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 20, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 21, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 22, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 23, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 24, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 25, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 26, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 27, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 28, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 29, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 30, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 31, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 32, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 33, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 34, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 35, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 36, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 37, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 38, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 39, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 40, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 41, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 42, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 43, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 44, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 45, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 46, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 47, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 48, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 49, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 50, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 51, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 52, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 53, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 54, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 55, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 56, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 57, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 58, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 59, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 60, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 61, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 62, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 63, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 64, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 65, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 66, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 67, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 68, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 69, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 70, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 71, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 72, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 73, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
    <View style={{ position: "absolute", left: 74, top: 146, width: 1, height: 16, backgroundColor: "#000" }} />
    <View style={{ position: "absolute", left: 75, top: 146, width: 1, height: 16, backgroundColor: "#fff" }} />
  </Scene>;
}
