// error: errors/role-caption.tsx: props: caption is the caption the Timeline gives a captionPresenter(), so the scene declares no props of that name
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export const controls = { props: { caption: string({ default: "Hi" }) } };

// The Timeline gives a presenter its caption.
export const role = captionPresenter();

export default function Scene() {
  return <View style={{ width: 8, height: 8 }} />;
}
