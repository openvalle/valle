import { expect, test } from "bun:test";
import { compareFrames, compareFloatPixels } from "./pixels.ts";
const frame = (data: number[]) => ({ width: data.length / 4, height: 1, data: Uint8Array.from(data) });
const rule = { rgb: 2, alpha: 2 };

test("alpha-only errors remain visible even when RGB is black", () => {
  const result = compareFrames(frame([0,0,0,0]), frame([0,0,0,255]), rule);
  expect(result.rgbMax).toBe(0); expect(result.alphaMax).toBe(255);
  expect(result.whiteMax).toBe(255); expect(result.pass).toBe(false);
});
test("unused transparent RGB is ignored but a missing small element fails", () => {
  expect(compareFrames(frame([255,0,0,0]), frame([0,255,255,0]), rule).pass).toBe(true);
  const reference = frame([0,0,0,255,255,255,255,255,0,0,0,255]);
  const actual = frame([0,0,0,255,0,0,0,255,0,0,0,255]);
  expect(compareFrames(reference, actual, rule).failedPixels).toBe(1);
  expect(compareFrames(reference, actual, rule, { name:"detail",x:1,y:0,width:1,height:1 }).pass).toBe(false);
});
test("wrong-frame motion and alpha changes cannot be hidden by averages", () => {
  expect(compareFrames(frame([255,0,0,255,0,0,0,0]),frame([0,0,0,0,255,0,0,255]),rule).pass).toBe(false);
  expect(compareFrames(frame([255,255,255,128]),frame([255,255,255,132]),rule).pass).toBe(false);
});
test("float comparison preserves HDR and rejects nonfinite values", () => {
  expect(compareFloatPixels([2,1], [3,1], 0.01).pass).toBe(false);
  expect(compareFloatPixels([2,1], [2.001,1], 0.01).pass).toBe(true);
  expect(compareFloatPixels([0], [NaN], 0).pass).toBe(false);
});
test("invalid frame layout and tolerances fail rather than producing an empty pass", () => {
  expect(() => compareFrames(frame([0,0,0,0]),frame([0,0,0,0,1,2,3,4]),rule)).toThrow();
  expect(() => compareFrames(frame([0,0,0,0]),frame([0,0,0,0]),{rgb:NaN,alpha:2})).toThrow();
});

test("identical partial-alpha pixels pass an exact comparison", () => {
  const image=frame([255,96,48,128,50,120,200,33]);
  expect(compareFrames(image,image,{rgb:0,alpha:0}).pass).toBe(true);
});
