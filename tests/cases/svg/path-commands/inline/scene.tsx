export const composition = { width: 340, height: 220, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 220, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={220}>
        <path d="M20 20h70v50h-70z" fill="#176bba" />
        <path d="M120 70 Q145 0 170 70 T220 70" fill="none" stroke="#16a085" strokeWidth="6" />
        <path
          d="M20 145 C40 75 80 75 100 145 S160 215 180 145"
          fill="none"
          stroke="#ad1457"
          strokeWidth="4"
        />
        <path d="M240 130 a30 20 0 1 1 60 0 a30 20 0 1 1 -60 0z" fill="#ffb74d" />
      </svg>
    </View>
  );
}
