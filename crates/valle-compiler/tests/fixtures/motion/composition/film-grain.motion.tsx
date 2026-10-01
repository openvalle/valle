export const composition = { width: 640, height: 360, fps: 30, duration: 2 };

export default function FilmGrain() {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
    <View key="gray" className="absolute" style={{
      left: 200, top: 100, width: 240, height: 160,
      backgroundColor: "#808080", filter: "film-grain(7 0.12 2px)",
    }} />
  </Scene>;
}
