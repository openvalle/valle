// error: errors/sequence.tsx:3: TypeError: stage rise is after intro, which is no stage before it…
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };
const timing = defineSequence({ enter: { duration: 1 }, rise: { after: "intro", duration: 1 } });

export default function Scene() {
  const { seconds } = useCurrentTime();
  return <View style={{ opacity: stageProgress(seconds, timing.rise) }} />;
}
