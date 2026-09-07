const SHORT = [{ id: "a", value: 1 }];
const LONG = [{ id: "a", value: 1 }, { id: "b", value: 2 }];

export default function RuntimeStoryTopology(ctx) {
  const data = ctx.hold.progress > 0.5 ? LONG : SHORT;
  return <Scene key="scene">{data.map((item) => <View key={item.id} style={{ width: 20, height: item.value * 20 }} />)}</Scene>;
}
