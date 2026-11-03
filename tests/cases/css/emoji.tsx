export const composition = { width: 256, height: 160, fps: 1, duration: 1 };

// Emoji take Noto Color Emoji, after the fonts the text names: alone, with a skin tone, as a
// flag and with a joiner, at two sizes and in a translucent color, which shows them as much; and
// characters a text font has, which the selector VS16 shows as emoji.
export default function Scene() {
  return (
    <View style={{ padding: 8, gap: 6, color: "#263238" }}>
      <Text style={{ fontSize: 24 }}>Hi 😀🎉 👍🏽</Text>
      <Text style={{ fontSize: 16 }}>Flag 🇸🇬 family 👨‍👩‍👧 ok ✅</Text>
      <Text style={{ fontSize: 16, color: "#26323880" }}>Half 🌈</Text>
      <Text style={{ fontSize: 16 }}>
        {"Text \u00A9 emoji "}
        <Span>{"\u00A9\uFE0F"}</Span>
        {" \u2764\uFE0F #\uFE0F\u20E3"}
      </Text>
    </View>
  );
}
