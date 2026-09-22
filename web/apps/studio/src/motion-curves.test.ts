import { expect, test } from "bun:test";
import { curveRequest, type MotionCurveInputs } from "./motion-curves.ts";

test("curve requests retain source frame rate and typed props", () => {
  const inputs = {
    context: { durationFrames: 100000, fps: { num: 30000, den: 1001 }, viewport: { width: 640, height: 360 },
      artifact: { controls: { props: { distance: { default: { kind: "number", value: 180 } } } } } },
    props: { distance: { kind: "value", value: 240 } }, selectedKey: null,
  } as unknown as MotionCurveInputs;
  const request = curveRequest(inputs, "child", 5.3, 100005);
  expect(request).toMatchObject({ node: "child", startFrame: 5, endFrame: 99999, maxPoints: 240,
    fps: "30000/1001", props: { distance: { kind: "number", value: 240 } } });
  const next = structuredClone(inputs); next.props.distance!.value = 80; next.context.fps = { num: 24, den: 1 };
  expect(curveRequest(next, "child", 5, 50)).not.toEqual(request);
  expect(request.props.distance).toEqual({ kind: "number", value: 240 });
});
