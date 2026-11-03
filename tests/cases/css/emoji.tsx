export const composition = { width: 256, height: 128, fps: 1, duration: 1 };

// Emoji take Noto Color Emoji, after the fonts the text names: alone, with a skin tone, as a
// flag and with a joiner, at two sizes and in a translucent color, which shows them as much.
export default function Scene() {
  return (
    <View style={{ padding: 8, gap: 6, color: "#263238" }}>
      <Text style={{ fontSize: 24 }}>Hi 😀🎉 👍🏽</Text>
      <Text style={{ fontSize: 16 }}>Flag 🇸🇬 family 👨‍👩‍👧 ok ✅</Text>
      <Text style={{ fontSize: 16, color: "#26323880" }}>Half 🌈</Text>
    </View>
  );
}
