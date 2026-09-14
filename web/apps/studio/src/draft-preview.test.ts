import { expect, test } from "bun:test";
import { DraftPreview } from "./draft-preview.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test("only the latest frozen draft is prepared after a busy request, including stale errors", async () => {
  for (const fail of [false, true]) {
    const first = deferred<string>();
    const prepared: number[] = [], applied: string[] = [], failures: unknown[] = [];
    let ready = 0;
    const queue = new DraftPreview<{ value: number }, string>({
      prepare: async (draft) => { prepared.push(draft.value); return prepared.length === 1 ? first.promise : String(draft.value); },
      apply: async (result) => { applied.push(result); },
      updating() {}, failed: (error) => { failures.push(error); }, ready: () => { ready++; },
    });
    queue.schedule({ value: 1 });
    const running = queue.flush();
    queue.schedule({ value: 2 });
    const latest = { value: 3 };
    queue.schedule(latest);
    latest.value = 4;
    if (fail) first.reject(new Error("old failure")); else first.resolve("old result");
    await running;
    expect(prepared).toEqual([1, 3]);
    expect(applied).toEqual(["3"]);
    expect(failures).toEqual([]);
    expect(ready).toBe(1);
    queue.invalidate();
  }
});

test("edits during resource staging invalidate admission and do not report synchronized", async () => {
  const stage = deferred<void>();
  let current!: () => boolean;
  const ready: number[] = [];
  const queue = new DraftPreview<number, number>({
    prepare: async (draft) => draft,
    apply: async (result, _draft, isCurrent) => { if (result === 1) { current = isCurrent; await stage.promise; } },
    updating() {}, failed(error) { throw error; }, ready: () => { ready.push(1); },
  });
  queue.schedule(1);
  const running = queue.flush();
  await Promise.resolve();
  expect(current()).toBe(true);
  queue.schedule(2);
  expect(current()).toBe(false);
  stage.resolve();
  await running;
  expect(ready).toHaveLength(1);
  queue.invalidate();
});

test("workspace disposal drops in-flight results; a failed draft can be retried", async () => {
  const response = deferred<number>();
  let applied = 0, failures = 0;
  const queue = new DraftPreview<number, number>({
    prepare: () => response.promise,
    apply: async () => { applied++; }, updating() {}, ready() {}, failed() { failures++; },
  });
  queue.schedule(1);
  const running = queue.flush();
  queue.invalidate();
  response.resolve(1);
  await running;
  expect(applied).toBe(0);
  expect(failures).toBe(0);
  let attempt = 0;
  const retry = new DraftPreview<number, number>({
    prepare: async (draft) => { if (++attempt === 1) throw new Error("failed"); return draft; },
    apply: async () => { applied++; }, updating() {}, ready() {}, failed() { failures++; },
  });
  retry.schedule(2); await retry.flush();
  retry.schedule(2); await retry.flush();
  expect(failures).toBe(1);
  expect(applied).toBe(1);
});
