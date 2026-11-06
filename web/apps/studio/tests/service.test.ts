import { afterAll, afterEach, beforeAll, beforeEach, expect, mock, test } from "bun:test";

let api: typeof import("../src/service.ts");
const originalFetch = globalThis.fetch;
const globals = ["location", "sessionStorage"].map(
  (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)] as const,
);
const fetch = mock(async (_url: string | URL | Request, _init?: RequestInit) => Response.json({}));

beforeAll(async () => {
  const storage = new Map([["valle-token", "old-token"]]);
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: new URL("http://localhost:3000/?token=test-token"),
  });
  Object.defineProperty(globalThis, "sessionStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
    },
  });
  api = await import("../src/service.ts");
});

beforeEach(() => {
  fetch.mockReset();
  fetch.mockResolvedValue(Response.json({}));
  globalThis.fetch = fetch as typeof globalThis.fetch;
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});
afterAll(() => {
  for (const [name, descriptor] of globals) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else Reflect.deleteProperty(globalThis, name);
  }
});

test("requests use the URL token, preserve headers and do not mutate the caller's headers", async () => {
  const headers = new Headers({ "x-valle-token": "wrong-token", "content-type": "text/plain" });
  await api.request("/api/files/a.tsx", { method: "PUT", body: "source", headers });
  const [url, init] = fetch.mock.calls[0];
  expect(url).toBe("/api/files/a.tsx");
  expect(init).toMatchObject({ method: "PUT", body: "source" });
  expect(new Headers(init?.headers).get("x-valle-token")).toBe("test-token");
  expect(new Headers(init?.headers).get("content-type")).toBe("text/plain");
  expect(headers.get("x-valle-token")).toBe("wrong-token");
  expect(sessionStorage.getItem("valle-token")).toBe("test-token");
});

test("file addresses encode each path segment while preserving folders", () => {
  const path = "démos/a #?%.tsx";
  expect(api.address(path)).toBe("/api/files/d%C3%A9mos/a%20%23%3F%25.tsx");
  expect(api.link(path)).toBe("/api/files/d%C3%A9mos/a%20%23%3F%25.tsx?token=test-token");
});

test.each([
  ["/api/files/a.tsx?version=2", "/api/files/a.tsx?version=2"],
  ["http://localhost:3000/api/files/a.tsx", "/api/files/a.tsx"],
  [
    "https://assets.test/a b.png?q=1&v=2",
    "/api/url?u=https%3A%2F%2Fassets.test%2Fa%2520b.png%3Fq%3D1%26v%3D2",
  ],
])("fetching %s uses the authenticated service", async (url, expected) => {
  await api.fetching(url);
  const [path, init] = fetch.mock.calls[0];
  expect(path).toBe(expected);
  expect(new Headers(init?.headers).get("x-valle-token")).toBe("test-token");
});

test("reading preserves the text and revision used for the next write", async () => {
  fetch.mockResolvedValue(
    new Response("const x = '你好';\n", { headers: { "x-valle-sha256": "revision-a" } }),
  );
  expect(await api.service.read("a.tsx")).toEqual({
    text: "const x = '你好';\n",
    sha256: "revision-a",
  });
});

test("a missing file is distinct from a failed read", async () => {
  fetch.mockResolvedValueOnce(new Response("missing", { status: 404 }));
  expect(await api.service.read("a.tsx")).toBeUndefined();
  fetch.mockResolvedValueOnce(new Response("access denied", { status: 403 }));
  await expect(api.service.read("a.tsx")).rejects.toThrow("a.tsx: access denied");
});

test.each([
  { name: "an existing file", base: "revision-a" },
  { name: "a new file", base: null },
])("writing checks the original revision of $name", async ({ base }) => {
  fetch.mockResolvedValue(Response.json({ sha256: "revision-b" }));
  expect(await api.service.write("a.tsx", "new source", base)).toBe("revision-b");
  const [url, init] = fetch.mock.calls[0];
  expect(url).toBe("/api/files/a.tsx");
  expect(init).toMatchObject({ method: "PUT", body: "new source" });
  expect(new Headers(init?.headers).get("x-valle-base")).toBe(base ?? "none");
});

test.each([
  { name: "changed", sha256: "other-revision" },
  { name: "deleted", sha256: null },
])("a concurrently $name file reports a conflict", async ({ sha256 }) => {
  fetch.mockResolvedValue(Response.json({ sha256 }, { status: 409 }));
  const error = await api.service.write("a.tsx", "mine", "old").catch((error) => error);
  expect(error).toBeInstanceOf(api.Conflict);
  expect(error.path).toBe("a.tsx");
  expect(error.sha256).toBe(sha256);
  expect(fetch).toHaveBeenCalledTimes(1);
});

test("a failed write is not mistaken for a conflict or retried without its revision", async () => {
  fetch.mockResolvedValue(Response.json({ message: "disk full" }, { status: 500 }));
  await expect(api.service.write("a.tsx", "mine", "old")).rejects.toThrow("disk full");
  expect(fetch).toHaveBeenCalledTimes(1);
});

test.each([
  { status: 403, body: { message: "unauthorized" }, expected: "unauthorized" },
  { status: 200, body: { status: "error", message: "no project" }, expected: "no project" },
  { status: 500, body: {}, expected: "/api/session: 500" },
])("service errors retain their cause: $expected", async ({ status, body, expected }) => {
  fetch.mockResolvedValue(Response.json(body, { status }));
  await expect(api.service.session()).rejects.toThrow(expected);
});

test("network failures propagate to the caller", async () => {
  const error = new TypeError("connection refused");
  fetch.mockRejectedValue(error);
  await expect(api.service.read("a.tsx")).rejects.toBe(error);
});
