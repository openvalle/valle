import { expect, test } from "bun:test";
import { exportFrames, type Opened } from "../src/messages.ts";

test.each([
  {
    name: "the composition's rate",
    fps: "30",
    frames: 3,
    duration: 0.1,
    rate: [30, 1],
    expected: [0, 1, 2],
  },
  {
    name: "an equivalent fraction",
    fps: "30000/1001",
    frames: 3,
    duration: 0.1001,
    rate: [60000, 2002],
    expected: [0, 1, 2],
  },
  {
    name: "double the rate",
    fps: "30",
    frames: 3,
    duration: 0.1,
    rate: [60, 1],
    expected: [0, 0, 1, 1, 2, 2],
  },
  { name: "half the rate", fps: "30", frames: 3, duration: 0.1, rate: [15, 1], expected: [0, 2] },
  {
    name: "a fractional output rate",
    fps: "60",
    frames: 6,
    duration: 0.1,
    rate: [30000, 1001],
    expected: [0, 2, 4],
  },
  {
    name: "a fractional source rate",
    fps: "30000/1001",
    frames: 3,
    duration: 0.1001,
    rate: [60, 1],
    expected: [0, 0, 0, 1, 1, 2, 2],
  },
  {
    name: "a partial final interval holds the last frame",
    fps: "30",
    frames: 3,
    duration: 0.101,
    rate: [60, 1],
    expected: [0, 0, 1, 1, 2, 2, 2],
  },
  {
    name: "floating-point noise adds no frame",
    fps: "30",
    frames: 9,
    duration: 0.1 + 0.2,
    rate: [60, 1],
    expected: [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8],
  },
])("export frames: $name", ({ fps, frames, duration, rate, expected }) => {
  const composition: Opened = {
    width: 100,
    height: 100,
    fps,
    frames,
    duration,
    loop: false,
    controls: { props: {}, data: {}, resources: {} },
  };
  expect(exportFrames(composition, rate as [number, number])).toEqual(expected);
});
