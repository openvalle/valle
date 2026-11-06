export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <defs>
          <clipPath id="hole" clipPathUnits="userSpaceOnUse" transform="translate(10 10)">
            <path clipRule="evenodd" d="M0 0H100V100H0Z M25 25H75V75H25Z" />
          </clipPath>
          <clipPath
            id="box"
            clipPathUnits="objectBoundingBox"
            transform="translate(125 40) scale(0.5)"
          >
            <rect width="1" height="1" />
          </clipPath>
        </defs>
        <g transform="translate(20 20)">
          <rect width="120" height="120" fill="#176bba" clipPath="url(#hole)" />
        </g>
        <rect x="180" y="20" width="140" height="120" fill="#16a085" clipPath="url(#box)" />
      </svg>
    </View>
  );
}
