import { describe, expect, test } from "bun:test";

import { browserExitError, pollBrowserReport, terminateBrowser, waitForUnexpectedBrowserExit } from "./browser.ts";
import { createTempDirectory, joinPath, removeDirectory } from "./files.ts";

describe("browser subprocess diagnostics", () => {
  test("classifies an early signal as a browser exit, not a timeout", () => {
    const error = browserExitError({
      operation: "browser smoke",
      binary: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      pageUrl: "http://127.0.0.1:40000/preview.html?smoke=1",
      exitCode: 134,
      signalCode: "SIGABRT",
      stderr: "fatal startup failure\n",
    });
    expect(error.message).toContain("browser process exited before reporting (signal SIGABRT, exit code 134)");
    expect(error.message).toContain("fatal startup failure");
    expect(error.message).not.toContain("timed out");
  });

  test("races a real early exit and includes the captured stderr tail", async () => {
    const bun = Bun.which("bun");
    if (!bun) throw new Error("bun executable is unavailable");
    const directory = createTempDirectory("valle-browser-exit-test");
    const stderrPath = joinPath(directory, "browser.stderr.log");
    try {
      const child = Bun.spawn({
        cmd: [bun, "-e", "console.error('synthetic browser crash'); process.exit(42)"],
        stdin: "ignore",
        stdout: "ignore",
        stderr: Bun.file(stderrPath),
      });
      const failure = waitForUnexpectedBrowserExit(child, {
        operation: "browser smoke",
        binary: bun,
        pageUrl: "http://127.0.0.1/smoke",
        stderrPath,
        isCompleted: () => false,
      });
      await expect(failure).rejects.toThrow("browser process exited before reporting (exit code 42)");
      await expect(failure).rejects.toThrow("synthetic browser crash");
    } finally {
      removeDirectory(directory);
    }
  });

  test("cancels a hosted report poll without leaving its retry timer alive", async () => {
    let markRequested!: () => void;
    const requested = new Promise<void>((resolve) => {
      markRequested = resolve;
    });
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch() {
        markRequested();
        return new Response("not ready", { status: 404 });
      },
    });
    const abort = new AbortController();
    try {
      const polling = pollBrowserReport(
        `http://127.0.0.1:${server.port}/smoke-report`,
        abort.signal,
        60_000,
      );
      await requested;
      abort.abort();
      await expect(polling).rejects.toMatchObject({ name: "AbortError" });
    } finally {
      abort.abort();
      server.stop(true);
    }
  });

  test("terminates Chrome gracefully and only escalates a hung child", async () => {
    const gracefulSignals: Array<number | undefined> = [];
    let resolveGraceful!: (code: number) => void;
    const gracefulExit = new Promise<number>((resolve) => { resolveGraceful = resolve; });
    const gracefulChild = {
      exitCode: null,
      exited: gracefulExit,
      kill(signal?: number) {
        gracefulSignals.push(signal);
        resolveGraceful(0);
      },
    } as unknown as Bun.Subprocess;
    expect(await terminateBrowser(gracefulChild, 10)).toBe("terminated");
    expect(gracefulSignals).toEqual([undefined]);

    const hungSignals: Array<number | undefined> = [];
    let resolveHung!: (code: number) => void;
    const hungExit = new Promise<number>((resolve) => { resolveHung = resolve; });
    const hungChild = {
      exitCode: null,
      exited: hungExit,
      kill(signal?: number) {
        hungSignals.push(signal);
        if (signal === 9) resolveHung(137);
      },
    } as unknown as Bun.Subprocess;
    expect(await terminateBrowser(hungChild, 1)).toBe("killed");
    expect(hungSignals).toEqual([undefined, 9]);
  });
});
