import { expect, test } from "bun:test";
import { waitForGpuCompletion } from "./gpu-completion.ts";

function context(status: number[]) {
  const sync = {}, retired: object[] = [];
  let flushes = 0;
  const gl = {
    SYNC_GPU_COMMANDS_COMPLETE: 1, ALREADY_SIGNALED: 2,
    CONDITION_SATISFIED: 3, WAIT_FAILED: 4,
    fenceSync: () => sync, flush: () => flushes++,
    clientWaitSync: () => status.length > 1 ? status.shift()! : status[0]!,
    deleteSync: (value: object) => retired.push(value),
  } as unknown as WebGL2RenderingContext;
  return { gl, sync, retired, flushes: () => flushes };
}

test("startup completion yields while GPU work is pending and releases its fence", async () => {
  const c = context([0, 0, 3]);
  let yielded = false;
  setTimeout(() => { yielded = true; }, 0);
  await waitForGpuCompletion(c.gl);
  expect(yielded).toBe(true);
  expect(c.flushes()).toBe(1);
  expect(c.retired).toEqual([c.sync]);
});

test("failed or stalled completion cannot retain a fence or hang startup", async () => {
  const failed = context([4]);
  await expect(waitForGpuCompletion(failed.gl)).rejects.toThrow("completion failed");
  expect(failed.retired).toEqual([failed.sync]);
  const stalled = context([0]);
  await expect(waitForGpuCompletion(stalled.gl, 1)).rejects.toThrow("timed out");
  expect(stalled.retired).toEqual([stalled.sync]);
});

test("an unavailable completion fence rejects before submitting more GPU work", async () => {
  const c = context([3]);
  c.gl.fenceSync = () => null;
  await expect(waitForGpuCompletion(c.gl)).rejects.toThrow("unavailable");
  expect(c.flushes()).toBe(0);
  expect(c.retired).toEqual([]);
});
