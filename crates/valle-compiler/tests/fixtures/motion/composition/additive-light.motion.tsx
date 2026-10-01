export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function AdditiveLight() {
  return <Scene style={{ width: 640, height: 360, backgroundColor: "#101010" }}>
    <View key="base" style={{ position: "absolute", width: 640, height: 360, backgroundColor: "#800000" }} />
    <View key="add" style={{ position: "absolute", width: 320, height: 360, backgroundColor: "#800000", mixBlendMode: "plus-lighter" }} />
  </Scene>;
}
