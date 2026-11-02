export const composition = { width: 320, height: 200, fps: 1, duration: 1 };

export const controls = {
  resources: {
    photo: resource({ kind: "image", default: "../resources/photo.png" }),
  },
};

// Filter classes: each function, of the theme, bare, negative and written, and several together;
// a filter written; backdrop filters of each function over stripes; drop shadows of the theme, in
// a color, at an opacity and written; and images fitted and placed.
export default function Scene() {
  const box = "size-7 bg-linear-to-br from-rose-500 to-sky-400";
  const functions = [
    "blur-xs",
    "brightness-150",
    "contrast-50",
    "grayscale",
    "hue-rotate-90",
    "-hue-rotate-30",
    "invert",
    "saturate-200",
    "sepia",
    "sepia-50",
    "brightness-75 contrast-125 saturate-150",
    "filter-[hue-rotate(45deg)_blur(1px)]",
  ];
  const backdrops = [
    "backdrop-blur-xs",
    "backdrop-brightness-50",
    "backdrop-contrast-200",
    "backdrop-grayscale",
    "backdrop-hue-rotate-90",
    "backdrop-invert",
    "backdrop-opacity-50",
    "backdrop-saturate-200",
    "backdrop-sepia",
    "backdrop-filter-[blur(1px)_invert(1)]",
  ];
  const fits = [
    "object-contain",
    "object-cover",
    "object-fill",
    "object-none",
    "object-scale-down",
  ];
  return (
    <View className="flex flex-wrap items-start gap-2 p-2 bg-neutral-100">
      {functions.map((f) => (
        <View key={f} className={`${box} ${f}`} />
      ))}
      <View className="flex w-72 bg-linear-to-r from-red-500 via-yellow-300 to-blue-600 p-1">
        {backdrops.map((b) => (
          <View key={b} className={`w-6 h-6 mr-1 ${b}`} />
        ))}
      </View>
      <View className={`${box} rounded-full drop-shadow-md`} />
      <View className={`${box} rounded-full drop-shadow-xl/50`} />
      <View className={`${box} rounded-full drop-shadow-lg drop-shadow-indigo-500`} />
      <View className={`${box} rounded-full drop-shadow-[2px_2px_0_#e91e63]`} />
      <View className={`${box} rounded-full drop-shadow-[0_0_3px_#1e88e5]/50`} />
      <View className={`${box} rounded-full drop-shadow-none`} />
      {fits.map((f) => (
        <Image
          key={f}
          src="resource://photo"
          className={`w-9 h-5 bg-amber-100 ${f} object-bottom-right`}
        />
      ))}
    </View>
  );
}
