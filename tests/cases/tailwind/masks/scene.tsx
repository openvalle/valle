export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

// Mask classes: each side's gradient, from and to a stop, in percent, of the spacing scale, of a
// color and written; two sides together, and both on an axis; linear gradients at angles; radial
// ones, shaped, sized and placed; conic ones; and a mask written, sized, placed and repeated, its
// box clipped and its origin moved.
export default function Scene() {
  const box = "size-10 bg-linear-to-br from-amber-300 via-rose-500 to-indigo-700";
  const masks = [
    "mask-t-from-50%",
    "mask-r-to-75%",
    "mask-b-from-2",
    "mask-l-from-[30%] mask-l-to-[90%]",
    "mask-x-from-70%",
    "mask-y-to-80%",
    "mask-t-from-50% mask-l-from-50%",
    "mask-b-from-black/50 mask-b-to-black",
    "mask-linear-45 mask-linear-from-20%",
    "-mask-linear-30 mask-linear-to-60%",
    "mask-linear-90 mask-linear-from-black/25 mask-linear-to-black",
    "mask-radial-from-40%",
    "mask-circle mask-radial-from-30% mask-radial-to-70%",
    "mask-radial-closest-side mask-radial-from-70%",
    "mask-radial-at-top-left mask-radial-from-20%",
    "mask-radial-[farthest-side] mask-radial-at-[25%_75%] mask-radial-to-80%",
    "mask-conic-from-50%",
    "mask-conic-90 mask-conic-from-25% mask-conic-to-75%",
    "mask-t-from-50% mask-radial-from-60%",
    "mask-[linear-gradient(90deg,#000,transparent)] mask-exclude",
    "mask-[radial-gradient(#000_60%,transparent_62%)] mask-size-[12px_12px] mask-repeat-x mask-top-left",
    "mask-[linear-gradient(#000,transparent)] mask-no-repeat mask-contain mask-center",
    "mask-[conic-gradient(#fff,#000,#fff)] mask-luminance",
    "mask-[radial-gradient(#000_60%,transparent_62%)] mask-repeat-space mask-size-[9px_9px]",
    "mask-[linear-gradient(#000,#000)] mask-origin-content mask-clip-padding mask-no-repeat mask-size-[50%_50%] border-4 p-1",
  ];
  return (
    <View className="flex flex-wrap items-start gap-2 p-2 bg-neutral-100">
      {masks.map((m) => (
        <View key={m} className={`${box} ${m}`} />
      ))}
    </View>
  );
}
