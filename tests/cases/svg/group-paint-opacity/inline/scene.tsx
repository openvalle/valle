export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <g transform="translate(20 20)" opacity="0.5">
          <rect
            x="4"
            y="4"
            width="100"
            height="100"
            fill="#176bba"
            stroke="#17324d"
            strokeWidth="8"
          />
          <g opacity="0.5">
            <rect x="40" y="40" width="100" height="100" fill="#ffb74d" />
          </g>
        </g>
        <g transform="translate(180 20)">
          <rect
            x="4"
            y="4"
            width="100"
            height="100"
            fill="#176bba"
            fillOpacity="0.5"
            stroke="#17324d"
            strokeWidth="8"
            strokeOpacity="0.5"
          />
          <rect x="40" y="40" width="100" height="100" fill="#ffb74d" fillOpacity="0.25" />
        </g>
      </svg>
    </View>
  );
}
