// error: errors/video-image.tsx:12:10: <Image> src resource://clip: clip is a video, not an image; a <Video> shows it
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

export const controls = {
  resources: {
    clip: resource({ kind: "video", default: "../resources/counter.mp4" }),
  },
};

// A video is shown by a <Video>.
export default function Scene() {
  return <Image src="resource://clip" style={{ width: 8, height: 8 }} />;
}
