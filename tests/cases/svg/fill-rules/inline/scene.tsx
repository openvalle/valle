export const composition = { width: 340, height: 180, fps: 1, duration: 1 };

export default function Scene() {
  return (
    <View style={{ width: 340, height: 180, backgroundColor: "white" }}>
      <svg key="drawing" width={340} height={180}>
        <path d="M20 20h120v120h-120z M50 50h60v60h-60z" fill="#176bba" fillRule="nonzero" />
        <path d="M180 20h120v120h-120z M210 50h60v60h-60z" fill="#16a085" fillRule="evenodd" />
      </svg>
    </View>
  );
}
