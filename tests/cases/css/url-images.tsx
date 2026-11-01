export const composition = { width: 360, height: 210, fps: 1, duration: 1 };

export const controls = {
  resources: {
    photo: resource({ kind: "image", default: "../resources/photo.png" }),
    tile: resource({ kind: "image", default: "../resources/tile.png" }),
  },
};

// url() images in backgrounds and masks: repeated at their natural size; a height alone, the
// width as the natural ratio has it, placed from the bottom right; fitting the box and covering
// it; rounded to whole tiles, the other side keeping their ratio; spaced out; over a gradient,
// under a dashed border with rounded corners; in the shorthand, quoted, over its color; a mask
// by the image's alpha, sized and repeated; and one by its luminance, covering the box. Where
// tiles touch, an edge pixel reads the next tile's. The tile with its hard edges is scaled by
// whole numbers: Chrome's raster reads between pixels in sixteenths.
export default function Scene() {
  const photo = "url(resource://photo)";
  const tile = "url(resource://tile)";
  const box = (width: number, style: object) => (
    <View style={{ width, height: 60, background: "#eceff1", ...style }} />
  );
  return (
    <View style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: 6 }}>
      {box(100, { backgroundImage: photo })}
      {box(80, {
        backgroundImage: photo,
        backgroundSize: "auto 20px",
        backgroundPosition: "right 4px bottom 4px",
        backgroundRepeat: "no-repeat",
      })}
      {box(70, {
        backgroundImage: photo,
        backgroundSize: "contain",
        backgroundRepeat: "no-repeat",
      })}
      {box(70, { backgroundImage: photo, backgroundSize: "cover", backgroundPosition: "center" })}
      {box(64, {
        backgroundImage: tile,
        backgroundSize: "18px auto",
        backgroundRepeat: "round repeat",
      })}
      {box(60, { backgroundImage: tile, backgroundSize: "16px 12px", backgroundRepeat: "space" })}
      {box(100, {
        backgroundImage: `${tile}, linear-gradient(135deg, #ffcc80, #4db6ac)`,
        backgroundSize: "16px auto, auto",
        border: "4px dashed #5c6bc0",
        borderRadius: 14,
      })}
      {box(90, { background: 'url("resource://photo") center / 50% no-repeat #f8bbd0' })}
      {box(60, { background: "#e53935", maskImage: tile, maskSize: "16px" })}
      {box(70, {
        background: "#1e88e5",
        maskImage: photo,
        maskSize: "cover",
        maskMode: "luminance",
      })}
    </View>
  );
}
