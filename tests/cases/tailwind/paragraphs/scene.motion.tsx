export const composition = { width: 320, height: 320, fps: 1, duration: 1 };

// Paragraph classes: indents of the scale, negative and written; justified lines; a clamp taken
// back; and smoothing, which changes nothing here.
export default function Scene() {
  const words = "The quick brown fox jumps over the lazy dog and runs into the woods";
  return (
    <View className="p-2 text-xs text-slate-800">
      <Text key="indent" className="mb-1 w-64 bg-slate-100 indent-4">
        {words}
      </Text>
      <Text key="negative" className="mb-1 ml-2 w-64 bg-slate-100 -indent-2">
        {words}
      </Text>
      <Text key="percentage" className="mb-1 w-64 bg-slate-100 indent-[10%]">
        {words}
      </Text>
      <Text key="justify" className="mb-1 w-64 bg-slate-100 text-justify">
        {words}
      </Text>
      <Text key="unclamped" className="mb-1 line-clamp-1 w-64 bg-slate-100 line-clamp-none">
        {words} {words}
      </Text>
      <Text key="smooth" className="w-64 bg-slate-100 antialiased">
        {words}
      </Text>
      <Text key="subpixel" className="w-64 bg-slate-100 subpixel-antialiased">
        {words}
      </Text>
    </View>
  );
}
