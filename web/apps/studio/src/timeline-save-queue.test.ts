import { expect, test } from "bun:test";

import { TimelineSaveQueue } from "./timeline-save-queue.ts";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

test("complete-document saves stay single-flight and pending edits use the new base", async () => {
  let base = "revision:a";
  let active = 0;
  let maximumActive = 0;
  const bases: string[] = [];
  const firstResponse = deferred();
  const queue = new TimelineSaveQueue(async () => {
    active += 1;
    maximumActive = Math.max(maximumActive, active);
    bases.push(base);
    if (bases.length === 1) await firstResponse.promise;
    base = bases.length === 1 ? "revision:b" : "revision:c";
    active -= 1;
    return true;
  });

  const first = queue.request();
  await Promise.resolve();
  queue.noteEdit();
  const joined = queue.request();
  expect(joined).toBe(first);
  firstResponse.resolve();
  await first;

  expect(bases).toEqual(["revision:a", "revision:b"]);
  expect(maximumActive).toBe(1);
  expect(queue.active).toBe(false);
});

test("a stale or rejected response stops without attempting a merge", async () => {
  let attempts = 0;
  let idle = 0;
  const response = deferred();
  const queue = new TimelineSaveQueue(async () => {
    attempts += 1;
    await response.promise;
    return false;
  }, () => { idle += 1; });

  const saving = queue.request();
  await Promise.resolve();
  queue.noteEdit();
  response.resolve();
  expect(await saving).toBe(false);
  expect(attempts).toBe(1);
  expect(idle).toBe(1);
});
