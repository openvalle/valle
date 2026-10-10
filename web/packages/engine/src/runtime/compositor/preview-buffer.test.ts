import { expect, test } from "bun:test";
import { PreviewBuffer } from "./preview-buffer.ts";
test("preview buffers bound ownership, discard clock-expired frames and drain", async () => {
  const made: number[] = [],
    closed: number[] = [];
  const q = new PreviewBuffer(
    0,
    5,
    3,
    async (frame) => {
      made.push(frame);
      return frame;
    },
    (v) => closed.push(v),
  );
  await q.prebuffer(3);
  expect(made).toEqual([0, 1, 2]);
  expect(await q.take(2)).toEqual({ frame: 2, value: 2 });
  expect(closed).toEqual([0, 1]);
  for (let f = 3; f <= 5; f++) expect((await q.take(f))?.frame).toBe(f);
  expect(await q.take(6)).toBeNull();
  q.close();
  await q.producer;
});
test("closing during creation releases late frames and stops production", async () => {
  let finish!: (v: number) => void;
  const closed: number[] = [];
  const q = new PreviewBuffer<number>(
    0,
    10,
    2,
    () => new Promise<number>((r) => (finish = r)),
    (v) => closed.push(v),
  );
  q.close();
  finish(0);
  await q.producer;
  expect(closed).toEqual([0]);
  expect(await q.take(0)).toBeNull();
});
test("producer failures reach prebuffer and playback without leaking ready frames", async () => {
  const failure = new Error("GPU capture failed"),
    closed: number[] = [];
  const q = new PreviewBuffer(
    0,
    1,
    2,
    async (frame) => {
      if (frame) throw failure;
      return frame;
    },
    (v) => closed.push(v),
  );
  expect(await q.prebuffer(2).catch((e) => e)).toBe(failure);
  q.close();
  expect(closed).toEqual([0]);
  await q.producer;
});

test("a clock jump skips obsolete work while preserving the bounded completed frames", async () => {
  let frame = 0;
  const made: number[] = [], disposed: number[] = [];
  const q = new PreviewBuffer(0, 20, 2, async (f) => {
    made.push(f);
    return f;
  }, f => disposed.push(f), () => frame);
  await q.prebuffer(2);
  expect(made).toEqual([0, 1]);
  frame = 18;
  expect((await q.take(18))?.frame).toBe(18);
  expect(made.slice(0, 3)).toEqual([0, 1, 18]);
  expect(made.some(f => f > 1 && f < 18)).toBe(false);
  expect(disposed).toEqual([0, 1]);
  expect((await q.take(19))?.frame).toBe(19);
  expect((await q.take(20))?.frame).toBe(20);
  expect(await q.take(21)).toBeNull();
  q.close();
  await q.producer;
});

test("an exact consumer can advance the producer without an external clock", async () => {
  const made: number[] = [];
  const q = new PreviewBuffer(0, 10, 1, async f => { made.push(f); return f; }, () => {});
  await q.prebuffer(1);
  expect((await q.take(10))?.frame).toBe(10);
  expect(made).toEqual([0, 10]);
  q.close();
  await q.producer;
});
