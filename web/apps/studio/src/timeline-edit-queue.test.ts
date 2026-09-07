import { expect, test } from "bun:test";

import { TimelineEditQueue } from "./timeline-edit-queue.ts";

test("Timeline edit queue serializes edits and survives one rejected edit", async () => {
  const queue = new TimelineEditQueue();
  const order: string[] = [];
  let releaseFirst!: () => void;
  const firstGate = new Promise<void>((resolve) => { releaseFirst = resolve; });

  queue.enqueue(async () => {
    order.push("first:start");
    await firstGate;
    order.push("first:end");
  });
  queue.enqueue(async () => {
    order.push("rejected");
    throw new Error("bad edit");
  }, (error) => {
    order.push((error as Error).message);
  });
  queue.enqueue(async () => {
    order.push("third");
  });

  await Promise.resolve();
  expect(order).toEqual(["first:start"]);
  releaseFirst();
  await queue.idle();
  expect(order).toEqual(["first:start", "first:end", "rejected", "bad edit", "third"]);
});
