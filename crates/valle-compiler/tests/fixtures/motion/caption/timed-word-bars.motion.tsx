export const composition = { width: 240, height: 80, fps: 30, duration: 1 };
export const role = captionPresenter({ intro: 0.15, outro: 0.15 });
export default function Words() { const time = useCurrentTime(); const caption = useCaption();
  return <Scene>{caption.runs.map((run, index) => <View key={index} className="absolute" style={{
    left: index * 30, width: 24, height: 12, top: time.seconds * 10,
    backgroundColor: time.clipSeconds >= run.start && time.clipSeconds < run.end ? "yellow" : "white",
  }} />)}</Scene>;
}
