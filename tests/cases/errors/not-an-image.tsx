// error: errors/not-an-image.tsx: resource logo: errors/not-an-image.tsx: not a PNG, JPEG or WebP image
export const composition = { width: 64, height: 64, fps: 1, duration: 1 };

// A resource whose file is not an image.
export const controls = {
  resources: {
    logo: resource({ kind: "image", default: "./not-an-image.tsx" }),
  },
};

export default function Scene() {
  return <Image src="resource://logo" />;
}
