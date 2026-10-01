export const composition = { width: 240, height: 80, fps: 30, duration: 1 };
export const role = captionPresenter({ intro: seconds(0.15), outro: seconds(0.15) });
export default function Words(ctx, props, data) {
  return <Scene>{data.runs.map((run, index) => <View key={index} className="absolute" style={{
    left: index * 30, width: 24, height: 12, top: ctx.seconds * 10,
    backgroundColor: ctx.host.seconds >= run.start && ctx.host.seconds < run.end ? "yellow" : "white",
  }} />)}</Scene>;
}
