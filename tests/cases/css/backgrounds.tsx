export const composition = { width: 256, height: 256, fps: 1, duration: 1 };

// Background images sized, placed and repeated: a checkerboard, stripes in tiles of fractional
// size and at a fractional position, one tile from a corner, two layers, tiles spaced out and
// rounded to fit, one spaced tile where its position puts it, a tile larger than the box,
// repeating down only, tiles under a rounded border, and the shorthand with a size.
export default function Scene() {
  const tile = (background: string, style: object = {}) => (
    <View style={{ width: 52, height: 36, background, ...style }} />
  );
  const stripes = "linear-gradient(to right, #e53935 50%, #ffcdd2 50%)";
  const dot = "radial-gradient(closest-side, #3949ab 70%, transparent 75%)";
  return (
    <View style={{ padding: 8, display: "flex", flexWrap: "wrap", gap: 8 }}>
      {tile(
        "linear-gradient(45deg, #3949ab 25%, transparent 25% 75%, #3949ab 75%), " +
          "linear-gradient(45deg, #3949ab 25%, transparent 25% 75%, #3949ab 75%), #e8eaf6",
        { backgroundSize: "12px 12px", backgroundPosition: "0 0, 6px 6px" },
      )}
      {tile(stripes, { backgroundSize: "10.5px 7.25px" })}
      {tile(stripes, { backgroundSize: "9px 9px", backgroundPosition: "3.5px 2.25px" })}
      {tile(`${dot}, #e8eaf6`, {
        backgroundSize: "20px 14px",
        backgroundPosition: "right 6px bottom 4px",
        backgroundRepeat: "no-repeat",
      })}
      {tile("linear-gradient(#00897b, #b2dfdb), linear-gradient(to right, #ff7043, #ffcc80)", {
        backgroundSize: "40% 50%, auto",
        backgroundPosition: "center, 0 0",
        backgroundRepeat: "no-repeat, repeat",
      })}
      {tile(`${dot}, #e8eaf6`, { backgroundSize: "15px 10px", backgroundRepeat: "space" })}
      {tile(`${dot}, #e8eaf6`, { backgroundSize: "15px 10px", backgroundRepeat: "round" })}
      {tile(`${stripes}, #e8eaf6`, { backgroundSize: "15px", backgroundRepeat: "round no-repeat" })}
      {tile(`${dot}, #e8eaf6`, {
        backgroundSize: "30px 20px",
        backgroundPosition: "75% 100%",
        backgroundRepeat: "space",
      })}
      {tile("radial-gradient(#8e24aa, #e1bee7)", {
        backgroundSize: "80px 50px",
        backgroundPosition: "center",
        backgroundRepeat: "no-repeat",
      })}
      {tile(`${stripes}, #e8eaf6`, {
        backgroundSize: "12px 8px",
        backgroundPosition: "50% 3px",
        backgroundRepeat: "repeat-y",
      })}
      {tile("linear-gradient(135deg, #1e88e5, #bbdefb)", {
        border: "6px solid rgb(0 0 0 / 30%)",
        borderRadius: 10,
        backgroundSize: "13px 11px",
      })}
      {tile("linear-gradient(#ffca28, #ef6c00) center / 50% 60% no-repeat, #fff3e0", {
        width: 112,
      })}
    </View>
  );
}
