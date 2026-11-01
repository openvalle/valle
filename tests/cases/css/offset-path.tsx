export const composition = { width: 256, height: 330, fps: 1, duration: 1 };

// Motion paths: arrows along a curve, turned with it, at a fixed angle and reversed; around a
// closed path past its end and back before its start, and stopped at an open one's ends; rays
// from the center of each size, from a point and from the box; a circle, an ellipse in the
// content box, a polygon, the rounded border box, and a circle around the arrow's own corner;
// anchors, a translate, rotate and transform around the move, and text on a path. Each arrow is
// off its containing block's top left corner, where Chrome 156 takes rays and shapes in a box
// of no size there.
export default function Scene() {
  const panel = (style: object, children: any) => (
    <View style={{ position: "relative", height: 80, background: "#e8eaf6", ...style }}>
      {children}
    </View>
  );
  const arrow = (style: object) => (
    <View
      style={{
        position: "absolute",
        left: 4,
        top: 4,
        width: 18,
        height: 8,
        borderRadius: "0 4px 4px 0",
        background: "linear-gradient(to right, #3949ab, #e57373)",
        ...style,
      }}
    />
  );
  const curve = "path('M10 50 C 60 -10, 140 100, 234 20')";
  const closed = "path('M10 10 H100 V60 H10 Z')";
  const open = "path('M10 72 H100')";
  return (
    <View style={{ padding: 6, display: "flex", flexDirection: "column", gap: 6 }}>
      {panel(
        { height: 70 },
        <>
          {[0, 20, 40, 60, 80, 100].map((d) =>
            arrow({ offsetPath: curve, offsetDistance: `${d}%` }),
          )}
          {arrow({
            offsetPath: curve,
            offsetDistance: "50%",
            offsetRotate: "0deg",
            background: "#43a047",
          })}
          {arrow({
            offsetPath: curve,
            offsetDistance: "30%",
            offsetRotate: "reverse",
            background: "#fdd835",
          })}
        </>,
      )}
      <View style={{ display: "flex", gap: 12 }}>
        {panel(
          { width: 116 },
          <>
            {["0%", "150%", "-10%"].map((d) => arrow({ offsetPath: closed, offsetDistance: d }))}
            {["150%", "-20%"].map((d) =>
              arrow({ offsetPath: open, offsetDistance: d, background: "#8e24aa" }),
            )}
          </>,
        )}
        {panel(
          { width: 116 },
          <>
            {arrow({ offsetPath: "ray(0deg)", offsetDistance: "100%" })}
            {arrow({ offsetPath: "ray(60deg sides)", offsetDistance: "100%" })}
            {arrow({ offsetPath: "ray(150deg closest-corner)", offsetDistance: "60%" })}
            {arrow({ offsetPath: "ray(250deg farthest-side)", offsetDistance: "80%" })}
            {arrow({
              offsetPath: "ray(30deg at 10px 70px)",
              offsetDistance: "30px",
              background: "#00897b",
            })}
            {arrow({
              left: 70,
              top: 50,
              offsetPath: "ray(-45deg)",
              offsetPosition: "auto",
              offsetDistance: "20px",
              background: "#e53935",
            })}
          </>,
        )}
      </View>
      {panel(
        { height: 90, padding: 6, borderRadius: 16 },
        <>
          {["0%", "30%", "60%"].map((d) =>
            arrow({ offsetPath: "circle(26px at 40px 45px)", offsetDistance: d }),
          )}
          {["10%", "45%"].map((d) =>
            arrow({
              offsetPath: "ellipse(34px 24px at 64px 34px) content-box",
              offsetDistance: d,
              background: "#00897b",
            }),
          )}
          {["0%", "40%", "70%"].map((d) =>
            arrow({
              offsetPath: "polygon(200px 10px, 236px 78px, 164px 78px)",
              offsetDistance: d,
              background: "#8e24aa",
            }),
          )}
          {["15%", "45%", "80%"].map((d) =>
            arrow({ offsetPath: "border-box", offsetDistance: d, background: "#fb8c00" }),
          )}
          {arrow({
            left: 120,
            top: 60,
            offsetPath: "circle(12px)",
            offsetPosition: "auto",
            offsetDistance: "20%",
            background: "#e53935",
          })}
        </>,
      )}
      {panel(
        { height: 60 },
        <>
          {arrow({ offsetPath: "path('M20 20 L230 20')", offsetAnchor: "0 0" })}
          {arrow({
            offsetPath: "path('M20 20 L230 20')",
            offsetDistance: "20%",
            offsetAnchor: "100% 100%",
            offsetRotate: "30deg",
          })}
          {arrow({
            offsetPath: "path('M20 40 L230 40')",
            offsetDistance: "45%",
            translate: "0 6px",
            rotate: "20deg",
            transform: "scale(1.5)",
            background: "#43a047",
          })}
          <Text
            style={{
              position: "absolute",
              fontSize: 14,
              color: "#283593",
              offsetPath: "path('M140 50 L230 10')",
              offsetDistance: "40%",
            }}
          >
            Valle
          </Text>
        </>,
      )}
    </View>
  );
}
