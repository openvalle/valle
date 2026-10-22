export const composition = { width: 640, height: 360, fps: 30, duration: 2 };
export default function EchoTrail() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
  return (
    <Scene className="relative h-full w-full" style={{ backgroundColor: "#101010" }}>
      <Echo key="echo" count={4} delay={0.1} decay={0.5}>
        <View key="dot" className="absolute" style={{ left: 50, top: 170, width: 20, height: 20,
          borderRadius: 10, backgroundColor: "#ffffff", translate: point(clamp(frame, 0, 30) * 10, 0) }} />
      </Echo>
    </Scene>
  );
}
