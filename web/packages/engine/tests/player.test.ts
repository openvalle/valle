import { expect, spyOn, test } from "bun:test";
import { createPlayer, Player, type Stats } from "../src/index.ts";
import type { Connection, Spawn } from "../src/connection.ts";
import type { Request } from "../src/messages.ts";

test("an ordinary module or JSON file is rejected before transferring the canvas", async () => {
  for (const entry of ["Card.tsx", "data.json", "timeline.json"]) {
    await expect(createPlayer({ canvas: {} as HTMLCanvasElement, entry })).rejects.toThrow(
      `${entry}: an entry must end with .motion.tsx or .timeline.json`,
    );
  }
});

// Drive the real player's scheduler with a controlled clock and worker replies.
async function playback(
  check: (p: {
    player: Player;
    stats: Stats[];
    drawn: number[];
    advance: (ms: number) => Promise<void>;
    latency: (ms: number) => void;
  }) => Promise<void>,
) {
  let now = 0;
  let latency = 0;
  let next = 0;
  const frames = new Map<number, FrameRequestCallback>();
  const replies: { at: number; done: () => void }[] = [];
  const drawn: number[] = [];
  const clock = spyOn(performance, "now").mockImplementation(() => now);
  const raf = Object.getOwnPropertyDescriptor(globalThis, "requestAnimationFrame");
  const caf = Object.getOwnPropertyDescriptor(globalThis, "cancelAnimationFrame");
  Object.assign(globalThis, {
    requestAnimationFrame: (callback: FrameRequestCallback) => {
      frames.set(++next, callback);
      return next;
    },
    cancelAnimationFrame: (id: number) => frames.delete(id),
  });
  const connection = {
    stopped: () => {},
    async send(request: Request) {
      if (request.type === "open")
        return {
          width: 16,
          height: 16,
          fps: "30",
          frames: 3000,
          duration: 100,
          controls: { props: {}, data: {}, resources: {} },
          loop: false,
        };
      if (request.type === "sound") return null;
      if (request.type === "frame") {
        drawn.push(request.index);
        if (latency) await new Promise<void>((done) => replies.push({ at: now + latency, done }));
        return { memory: 1024 };
      }
    },
    async close() {},
  };
  const player = new Player(
    connection as unknown as Connection,
    {
      entry: "test.motion.tsx",
      inputs: { props: {}, data: {}, resources: {} },
      given: {},
    } as Spawn,
    { canvas: {} as HTMLCanvasElement, entry: "test.motion.tsx" },
  );
  const stats: Stats[] = [];
  player.on("stats", (value) => stats.push(value));
  // Settle a worker reply and the scheduler's chained completion handlers.
  const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  };
  const advance = async (ms: number) => {
    now += ms;
    for (const reply of replies.splice(0)) {
      if (reply.at <= now) reply.done();
      else replies.push(reply);
    }
    await flush();
    const callbacks = [...frames.values()];
    frames.clear();
    for (const callback of callbacks) callback(now);
    await flush();
  };
  try {
    await player.start({});
    await check({ player, stats, drawn, advance, latency: (ms) => (latency = ms) });
  } finally {
    await player.close();
    clock.mockRestore();
    for (const [key, descriptor] of [
      ["requestAnimationFrame", raf],
      ["cancelAnimationFrame", caf],
    ] as const) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
}

test("playback FPS counts video frames, not faster display refreshes or paused draws", async () => {
  await playback(async ({ player, stats, advance }) => {
    for (let i = 0; i < 10; i++) await player.draw(0);
    await advance(10_000);
    player.play();
    // 125 browser callbacks per second, for a 30 fps composition.
    for (let i = 0; i < 125; i++) await advance(8);
    const sample = stats[0]!;
    expect(sample.elapsedMs).toBe(1000);
    expect(sample.shown).toBeGreaterThanOrEqual(28);
    expect(sample.shown).toBeLessThanOrEqual(30);
    expect(sample.fps).toBe(sample.shown);
    expect(sample.dropped).toBe(0);
    expect(sample.memory).toBe(1024);
  });
});

test("slow worker frames lower FPS and report skipped frames and latency", async () => {
  await playback(async ({ player, stats, advance, latency }) => {
    latency(100);
    player.play();
    for (let i = 0; i < 125; i++) await advance(8);
    const sample = stats[0]!;
    expect(sample.fps).toBeGreaterThan(0);
    expect(sample.fps).toBeLessThanOrEqual(10);
    expect(sample.dropped).toBeGreaterThan(0);
    expect(sample.frameMs).toBeGreaterThanOrEqual(100);
  });
});

test("a faster display submits each video frame only once while seeks still redraw", async () => {
  await playback(async ({ player, drawn, advance }) => {
    drawn.length = 0;
    player.play();
    for (let i = 0; i < 125; i++) await advance(8);
    expect(drawn.length).toBeGreaterThanOrEqual(29);
    expect(drawn.length).toBeLessThanOrEqual(30);
    expect(new Set(drawn).size).toBe(drawn.length);
    player.pause();
    const last = drawn.at(-1)!;
    player.seekFrame(last);
    await advance(1);
    expect(drawn.at(-1)).toBe(last);
    expect(drawn.filter((n) => n === last)).toHaveLength(2);
  });
});

test("seeking and resuming restart the sample without treating jumps or idle time as drops", async () => {
  await playback(async ({ player, stats, advance }) => {
    player.play();
    for (let i = 0; i < 50; i++) await advance(10);
    player.seekFrame(900);
    for (let i = 0; i < 50; i++) await advance(10);
    player.pause();
    expect(stats.at(-1)!.elapsedMs).toBe(500);
    expect(stats.at(-1)!.dropped).toBe(0);
    await advance(10_000);
    player.play();
    for (let i = 0; i < 50; i++) await advance(10);
    player.pause();
    const sample = stats.at(-1)!;
    expect(sample.elapsedMs).toBe(500);
    expect(sample.fps).toBeGreaterThanOrEqual(28);
    expect(sample.fps).toBeLessThanOrEqual(32);
    expect(sample.dropped).toBe(0);
  });
});
