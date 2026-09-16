import { expect, test } from "bun:test";
import { curveRequest, type MotionCurveInputs } from "./motion-curves.ts";

test("curve requests retain exact frame rate, typed props, phase and cue windows", () => {
  const inputs = {
    context: { durationFrames: 100000, fps: { num: 30000, den: 1001 }, viewport: { width: 640, height: 360 },
      artifact: { controls: { props: { distance: { default: { kind: "number", value: 180 } } }, timing: { holdCycleFrames: { default: 30 } } } } },
    props: { distance: { kind: "value", value: 240 } }, timing: { enterFrames: 12, exitFrames: 6 },
    cues: { beat: { type: "sourceRange", startFrame: 20, endFrame: 80, enterFrames: 4, exitFrames: 4 } }, selectedKey: null,
  } as unknown as MotionCurveInputs;
  const request = curveRequest(inputs, "child", 5.3, 100005);
  expect(request).toMatchObject({ node: "child", startFrame: 5, endFrame: 99999, maxPoints: 240, fps: "30000/1001",
    props: { distance: { kind: "number", value: 240 } }, phases: { enterFrames: 12, exitFrames: 6, holdCycleFrames: 30 },
    cues: { beat: { startFrame: 20, endFrame: 80, enterFrames: 4, exitFrames: 4 } } });
  const next = structuredClone(inputs); next.props.distance!.value = 80; next.context.fps = { num: 24, den: 1 };
  next.cues.beat!.startFrame = 30;
  expect(curveRequest(next, "child", 5, 50)).not.toEqual(request);
  expect(request.props.distance).toEqual({ kind: "number", value: 240 });
});
