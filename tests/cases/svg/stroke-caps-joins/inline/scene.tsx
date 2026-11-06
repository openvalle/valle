export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <path d="M30 30h90" fill="none" stroke="#176bba" strokeWidth="12" strokeLinecap="butt" />
        <path d="M30 65h90" fill="none" stroke="#16a085" strokeWidth="12" strokeLinecap="round" />
        <path d="M30 100h90" fill="none" stroke="#ffb74d" strokeWidth="12" strokeLinecap="square" />
        <path
          d="M170 110l20 -60l20 60"
          fill="none"
          stroke="#176bba"
          strokeWidth="10"
          strokeLinejoin="miter"
        />
        <path
          d="M225 110l20 -60l20 60"
          fill="none"
          stroke="#176bba"
          strokeWidth="10"
          strokeLinejoin="round"
        />
        <path
          d="M280 110l20 -60l20 60"
          fill="none"
          stroke="#176bba"
          strokeWidth="10"
          strokeLinejoin="bevel"
        />
      </svg>
    </View>
  );
}
