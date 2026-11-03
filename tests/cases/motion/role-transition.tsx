export const composition = { width: 160, height: 90, fps: 4, duration: 1 };

// A transition reads the clips it goes between as input://from and input://to: alone,
// placeholders of the composition's size. The first turns away as a card, and the second turns
// in behind it.
export const role = transition();

export default function Scene() {
  const { progress } = useCurrentTime();
  const turn = interpolate(progress, [0, 1], [0, 180]);
  const face = { position: "absolute", inset: 0, backfaceVisibility: "hidden" } as const;
  return (
    <View style={{ width: 160, height: 90, background: "#212121", perspective: 400 }}>
      <View
        style={{
          position: "absolute",
          inset: 0,
          transformStyle: "preserve-3d",
          transform: `rotateY(${turn}deg)`,
        }}
      >
        <Image src="input://from" style={face} />
        <Image src="input://to" style={{ ...face, transform: "rotateY(180deg)" }} />
      </View>
    </View>
  );
}
