export const composition = { width: 560, height: 360, fps: 1, duration: 1 };
export const controls = {
  resources: {
    Latin: resource({ kind: "font", default: "/tests/resources/NotoSans.ttf" }),
    Hebrew: resource({ kind: "font", default: "/tests/resources/NotoSansHebrew.ttf" }),
  },
};
export default function Scene() {
  return (
    <View style={{ width: 560, height: 360, backgroundColor: "white" }}>
      <svg width="560" height="360" fontFamily="Latin, Hebrew" fontSize="24" fill="#17365d">
        <path d="M180 10V330 M350 130V170" fill="none" stroke="#ddd" />
        <text x="180" y="35" direction="rtl">
          ABC 123
        </text>
        <text x="180" y="75" direction="rtl" unicodeBidi="bidi-override">
          ABC 123
        </text>
        <text x="180" y="115" direction="rtl" unicodeBidi="bidi-override" textAnchor="middle">
          ABC 123
        </text>
        <text x="180" y="155" direction="rtl" unicodeBidi="bidi-override">
          AB<tspan x="350">CD</tspan>
        </text>
        <text x="80 140 200" y="195" direction="rtl" unicodeBidi="bidi-override">
          ABC
        </text>
        <text x="180" y="235" direction="rtl">
          אבג 123
        </text>
        <text x="20" y="275">
          LEFT [
          <tspan direction="rtl" unicodeBidi="embed">
            אבג 123
          </tspan>
          ] RIGHT
        </text>
        <text
          x="180"
          y="315"
          direction="rtl"
          unicodeBidi="bidi-override"
          textLength="120"
          lengthAdjust="spacingAndGlyphs"
        >
          ABC 123
        </text>
      </svg>
    </View>
  );
}
