export const composition = { width: 256, height: 160, fps: 1, duration: 1 };

// Characters the family list has no glyph for fall back as Chrome's do, here to the built-in
// fonts of their scripts: Han, kana and bopomofo to Noto Sans SC, Hangul to Noto Sans KR; Latin
// stays in the list's fonts.
export default function Scene() {
  return (
    <View style={{ padding: 8, gap: 6, color: "#263238" }}>
      <Text style={{ fontSize: 18 }}>Valle 视频编辑 かなカナ ㄅㄆ</Text>
      <Text style={{ fontFamily: "serif", fontSize: 18, color: "#1e88e5" }}>
        Serif 字幕 한국어 텍스트
      </Text>
      <Text style={{ fontSize: 16, fontWeight: 700, color: "#e53935" }}>粗体 굵게 Bold</Text>
    </View>
  );
}
