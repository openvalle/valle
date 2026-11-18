import { expect, test } from "bun:test";
import { READ_WINDOW, Remote } from "../src/remote.ts";

test("ranged reads bound open-ended requests and preserve the server's bytes", async () => {
  const response = new Response("data", {
    status: 206,
    headers: { "Content-Range": "bytes 4-7/8" },
  });
  const remote = new Remote(async (_input, init) => {
    expect(new Headers(init?.headers).get("Range")).toBe(`bytes=4-${4 + READ_WINDOW - 1}`);
    return response;
  });
  try {
    const part = await remote.fetch("https://example.com/clip", { headers: { Range: "bytes=4-" } });
    expect(part).toBe(response);
    expect(await part.text()).toBe("data");
  } finally {
    await remote.dispose();
  }
});

test("a whole response that fits in the demuxer cache works without OPFS", async () => {
  const response = new Response("small", { headers: { "Content-Length": "5" } });
  const remote = new Remote(async () => response);
  try {
    expect(await remote.fetch("https://example.com/clip", { headers: { Range: "bytes=0-" } })).toBe(
      response,
    );
  } finally {
    await remote.dispose();
  }
});
