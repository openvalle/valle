export const composition = { width: 300, height: 210, fps: 4, duration: 2 };
export default function Scene() {
  const { seconds } = useCurrentTime();
  return (
    <View style={{ width: 300, height: 210, backgroundColor: "white" }}>
      <svg xmlns="http://www.w3.org/2000/svg" width="300" height="210" viewBox="0 0 300 210">
        <defs>
          <pattern id="p" width="20" height="20" patternUnits="userSpaceOnUse">
            <rect width="20" height="20" fill="#e0f2fe" />
            <circle cx="10" cy="10" r="5" fill="#0284c7" />
          </pattern>
          <pattern
            id="offset"
            href="#p"
            x="7"
            y="3"
            patternTransform={`rotate(${15 + seconds * 30})`}
          />
          <pattern id="bbox" width="0.25" height="0.5" patternContentUnits="objectBoundingBox">
            <rect width="0.25" height="0.5" fill="#fef3c7" />
            <circle cx="0.125" cy="0.25" r="0.06" fill="#d97706" />
          </pattern>
          <pattern id="vb" width="0.25" height="0.5" viewBox="0 0 10 10">
            <path d="M0 0H10V10H0Z" fill="#dcfce7" />
            <path d="M0 0L10 10M10 0L0 10" stroke="#16a34a" strokeWidth="2" />
          </pattern>
          <pattern id="inherit" width="18" height="18" patternUnits="userSpaceOnUse" fill="#ef4444">
            <rect width="8" height="8" fill="#7c3aed" />
          </pattern>
          <pattern id="copy" href="#inherit" fill="#7c3aed" />
        </defs>
        <rect x="10" y="10" width="80" height="80" fill="url(#p)" />
        <rect x="110" y="10" width="80" height="80" fill="url(#offset)" />
        <rect x="210" y="10" width="80" height="80" fill="url(#bbox)" />
        <rect x="10" y="110" width="80" height="80" fill="url(#vb)" />
        <circle cx="150" cy="150" r="32" fill="none" stroke="url(#p)" strokeWidth="14" />
        <rect x="210" y="110" width="80" height="80" fill="url(#copy)" fillOpacity="0.7" />
      </svg>
    </View>
  );
}
