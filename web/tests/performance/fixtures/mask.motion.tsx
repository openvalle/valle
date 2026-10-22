export const composition = { width: 640, height: 360, fps: 60, duration: 10 };

export default function IrisMask() { const time = useCurrentTime();
  return <Scene style={{ width: 640, height: 360, backgroundColor: "#101010" }}>
    <Mask key="iris" rect={rect(0, 0, 640, 360)} mode="alpha"
      style={{ position: "absolute", width: 640, height: 360 }}>
      <View key="content" style={{ width: 640, height: 360, backgroundColor: "#2140ff" }} />
      <MaskSource key="source">
        <Circle key="hole" cx={320} cy={180} r={10 + time.progress * 300} fill="#ffffff" />
      </MaskSource>
    </Mask>
  </Scene>;
}
