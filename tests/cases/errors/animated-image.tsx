// error: errors/animated-image.tsx: resource clip: resources/animated.webp: an animated WebP is not supported yet
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// An animated image, which the engine does not play yet.
export const controls = {
  resources: {
    clip: resource({ kind: "image", default: "../resources/animated.webp" }),
  },
};

export default function Scene() {
  return <Image src="resource://clip" />;
}
