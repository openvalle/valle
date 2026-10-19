export const composition = { width: 160, height: 96, fps: 30, duration: 2 };

// Smooth ramps: a sample one pixel from the expected one stays within the parity tolerance, while
// noise or vectors that are off land several pixels away. High frequencies and octaves reach
// lattice coordinates where noise computed in half precision is visibly wrong, and the views'
// edges reach the negative ones. The turned view is larger than its clip, so its antialiased
// edges, which Ganesh and Raster cover differently, stay out of sight.
const RAMP = "linear-gradient(135deg, rgb(50 70 210), rgb(180 180 80))";

export default function NoiseDisplacement(ctx) {
  return <Scene className="relative h-full w-full" style={{ backgroundColor: "#101820" }}>
    <View key="fractal" className="absolute" style={{
      left: 4, top: 6, width: 72, height: 84, backgroundImage: RAMP,
      displacement: displacement(7, point(0.21, 0.17), 24 + ctx.progress * 8, { octaves: 4, mode: "fractal" }),
    }} />
    <View key="clip" className="absolute overflow-hidden" style={{ left: 84, top: 6, width: 72, height: 84 }}>
      <View key="turbulence" className="absolute" style={{
        left: -64, top: -58, width: 200, height: 200, backgroundImage: RAMP, transform: "rotate(12deg) scale(1.1)",
        displacement: displacement(4000000, point(0.13, 0.29), 20, { octaves: 3, mode: "turbulence" }),
      }} />
    </View>
  </Scene>;
}
