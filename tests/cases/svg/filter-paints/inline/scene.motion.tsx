export const composition = { width: 440, height: 240, fps: 1, duration: 1 };
export default function Scene() {
  return (
    <View style={{ width: 440, height: 240, backgroundColor: "white" }}>
      <svg width="440" height="240">
        <defs>
          <linearGradient id="grad">
            <stop stopColor="#2878b8" />
            <stop offset="1" stopColor="#d06030" />
          </linearGradient>
          <pattern id="dots" width="12" height="12" patternUnits="userSpaceOnUse">
            <rect width="6" height="12" fill="#309060" />
          </pattern>
          <filter id="fill" x="-25%" y="-25%" width="150%" height="150%">
            <feOffset in="FillPaint" />
          </filter>
          <filter id="stroke" x="-25%" y="-25%" width="150%" height="150%">
            <feOffset in="StrokePaint" />
          </filter>
          <filter id="cut">
            <feComposite in="FillPaint" in2="SourceAlpha" operator="in" />
          </filter>
          <filter id="blur" x="0" y="0" width="100%" height="100%">
            <feGaussianBlur in="FillPaint" stdDeviation="3" />
          </filter>
        </defs>
        <circle cx="55" cy="55" r="26" fill="#2878b8" fillOpacity="0.5" filter="url(#fill)" />
        <circle
          cx="160"
          cy="55"
          r="26"
          fill="none"
          stroke="#d06030"
          strokeOpacity="0.5"
          strokeWidth="2"
          filter="url(#stroke)"
        />
        <circle cx="265" cy="55" r="26" fill="url(#grad)" filter="url(#fill)" />
        <circle cx="370" cy="55" r="26" fill="url(#dots)" filter="url(#fill)" />
        <circle cx="55" cy="165" r="26" fill="url(#grad)" filter="url(#cut)" />
        <rect
          x="125"
          y="140"
          width="60"
          height="48"
          fill="currentColor"
          color="#309060"
          filter="url(#fill)"
        />
        <rect x="230" y="140" width="60" height="48" fill="none" filter="url(#fill)" />
        <rect x="335" y="140" width="60" height="48" fill="#2878b8" filter="url(#blur)" />
      </svg>
    </View>
  );
}
