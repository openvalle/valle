export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <linearGradient
            id="pad"
            gradientUnits="userSpaceOnUse"
            x1="60"
            y1="0"
            x2="100"
            y2="0"
            spreadMethod="pad"
          >
            <stop offset="0" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#ffb74d" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
          <linearGradient
            id="repeat"
            gradientUnits="userSpaceOnUse"
            x1="60"
            y1="0"
            x2="100"
            y2="0"
            spreadMethod="repeat"
          >
            <stop offset="0" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#ffb74d" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
          <linearGradient
            id="reflect"
            gradientUnits="userSpaceOnUse"
            x1="60"
            y1="0"
            x2="100"
            y2="0"
            spreadMethod="reflect"
          >
            <stop offset="0" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#176bba" />
            <stop offset="0.5" stopColor="#ffb74d" />
            <stop offset="1" stopColor="#ffb74d" />
          </linearGradient>
        </defs>
        <rect x="20" y="20" width="280" height="35" fill="url(#pad)" />
        <rect x="20" y="75" width="280" height="35" fill="url(#repeat)" />
        <rect x="20" y="130" width="280" height="35" fill="url(#reflect)" />
      </svg>
    </View>
  );
}
