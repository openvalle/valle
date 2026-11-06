import { expect, mock, test } from "bun:test";
import { Connection, PlayerError } from "../src/connection.ts";

// Only the browser's transport is replaced; the real connection owns and settles requests.
class WorkerPort {
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  postMessage = mock((_message: unknown, _transfer: Transferable[] = []) => {});
  terminate = mock(() => {});

  receive(data: unknown) {
    this.onmessage?.({ data } as MessageEvent);
  }

  connect(fetch?: (url: string) => Promise<Response>) {
    return new Connection(this as unknown as Worker, fetch);
  }
}

test("worker replies settle their own request even when received out of order", async () => {
  const worker = new WorkerPort();
  const connection = worker.connect();
  const bytes = new Uint8Array([1, 2]);
  const first = connection.send({ type: "give", files: { "a.png": bytes } }, [bytes.buffer]);
  const second = connection.send({ type: "pixels" });
  expect(worker.postMessage.mock.calls[0]).toEqual([
    { id: 0, type: "give", files: { "a.png": bytes } },
    [bytes.buffer],
  ]);
  worker.receive({ id: 1, ok: true, value: bytes });
  await expect(second).resolves.toBe(bytes);
  worker.receive({ id: 0, ok: true });
  await expect(first).resolves.toBeUndefined();
  expect(() => worker.receive({ id: 1, ok: true, value: "late duplicate" })).not.toThrow();
});

test("a failed request preserves its error code and does not fail another request", async () => {
  const worker = new WorkerPort();
  const connection = worker.connect();
  const failed = connection.send({ type: "frame", index: 10 });
  const other = connection.send({ type: "pixels" });
  worker.receive({ id: 0, ok: false, code: "file", message: "image missing" });
  await expect(failed).rejects.toMatchObject({
    name: "PlayerError",
    code: "file",
    message: "image missing",
  });
  worker.receive({ id: 1, ok: true, value: 42 });
  await expect(other).resolves.toBe(42);
});

test("progress goes only to the matching pending request", async () => {
  const worker = new WorkerPort();
  const connection = worker.connect();
  const progress = mock((_done: number) => {});
  const pending = connection.send({ type: "pixels" }, [], progress);
  worker.receive({ progress: 9, done: 99 });
  worker.receive({ progress: 0, done: 2 });
  worker.receive({ id: 0, ok: true });
  await pending;
  worker.receive({ progress: 0, done: 3 });
  expect(progress.mock.calls).toEqual([[2]]);
});

test("a worker crash fails all pending requests and notifies the player", async () => {
  const worker = new WorkerPort();
  const connection = worker.connect();
  const stopped = mock((_error: PlayerError) => {});
  connection.stopped = stopped;
  const pending = Promise.allSettled([
    connection.send({ type: "pixels" }),
    connection.send({ type: "frame", index: 0 }),
  ]);
  const preventDefault = mock(() => {});
  worker.onerror!({ message: "device lost", preventDefault } as unknown as ErrorEvent);
  const error = stopped.mock.calls[0][0];
  expect(error).toBeInstanceOf(PlayerError);
  expect(error.code).toBe("scene");
  expect(error.message).toContain("device lost");
  expect(await pending).toEqual([
    { status: "rejected", reason: error },
    { status: "rejected", reason: error },
  ]);
  expect(preventDefault).toHaveBeenCalledTimes(1);
  expect(stopped).toHaveBeenCalledTimes(1);
});

test("closing releases the worker and rejects requests left unanswered", async () => {
  const worker = new WorkerPort();
  const connection = worker.connect();
  const pending = Promise.allSettled([connection.send({ type: "pixels" })]);
  const closed = connection.close();
  expect(worker.postMessage.mock.calls[1][0]).toEqual({ id: 1, type: "destroy" });
  worker.receive({ id: 1, ok: true });
  await closed;
  expect(await pending).toMatchObject([
    { status: "rejected", reason: { code: "scene", message: "the player was closed" } },
  ]);
  expect(worker.terminate).toHaveBeenCalledTimes(1);
});

test.each([200, 404])(
  "proxied fetch preserves HTTP %i and transfers only successful bodies",
  async (status) => {
    const worker = new WorkerPort();
    const fetch = mock(async (_url: string) => new Response(new Uint8Array([5, 9]), { status }));
    worker.connect(fetch);
    const sent = Promise.withResolvers<{ message: any; transfer: Transferable[] }>();
    worker.postMessage.mockImplementation((message, transfer) => {
      sent.resolve({ message, transfer });
    });
    worker.receive({ fetch: 12, url: "https://example.test/image.png" });
    const { message, transfer } = await sent.promise;
    expect(fetch).toHaveBeenCalledWith("https://example.test/image.png");
    expect(message).toMatchObject({ fetched: 12, status });
    if (status === 200) {
      expect(new Uint8Array(message.bytes)).toEqual(new Uint8Array([5, 9]));
      expect(transfer).toEqual([message.bytes]);
    } else {
      expect(message.bytes).toBeUndefined();
      expect(transfer).toEqual([]);
    }
  },
);

test("a proxy failure replies to the worker instead of leaving it waiting", async () => {
  const worker = new WorkerPort();
  worker.connect(async () => {
    throw new Error("offline");
  });
  const sent = Promise.withResolvers<unknown>();
  worker.postMessage.mockImplementation((message) => {
    sent.resolve(message);
  });
  worker.receive({ fetch: 4, url: "https://example.test/font.ttf" });
  expect(await sent.promise).toEqual({ fetched: 4, error: "Error: offline" });
});
