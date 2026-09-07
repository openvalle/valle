// Headless-browser WebCodecs decode orchestration using Bun.serve and Bun.spawn.

import { createTempDirectory, removeDirectory, repositoryPath } from "./files.ts";

const DECODE_PAGE = repositoryPath("web/tests/parity/assets/webcodecs-decode.html");
const PLAYER_CORE_JS = repositoryPath("web/dist/packages/player-core/index.mjs");
const MAC_CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PATH_CANDIDATES = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"] as const;

interface BrowserDecodeOptions {
  timeoutMs?: number;
}

interface BrowserWireFrame {
  timeS: number;
  ptsS: number;
  width: number;
  height: number;
  displayWidth: number;
  displayHeight: number;
  rgbaBase64: string;
}

interface BrowserDecodeWireResult {
  status: string;
  message?: string;
  implementation: string;
  userAgent: string;
  frames?: BrowserWireFrame[];
}

export interface BrowserDecodedFrame extends Omit<BrowserWireFrame, "rgbaBase64"> {
  data: Uint8Array;
}

export interface BrowserDecodeResult {
  implementation: string;
  userAgent: string;
  frames: BrowserDecodedFrame[];
}

export function resolveBrowser(): string | null {
  const override = process.env.VALLE_WEB_PARITY_BROWSER;
  if (override) return Bun.which(override);
  if (process.platform === "darwin") {
    const chrome = Bun.which(MAC_CHROME);
    if (chrome) return chrome;
  }
  for (const name of PATH_CANDIDATES) {
    const candidate = Bun.which(name);
    if (candidate) return candidate;
  }
  return null;
}

export interface BrowserExitContext {
  operation: string;
  binary: string;
  pageUrl: string;
  exitCode: number | null;
  signalCode: string | number | null;
  stderr: string;
}

export function browserExitError(context: BrowserExitContext): Error {
  const termination = context.signalCode == null
    ? `exit code ${context.exitCode ?? "unknown"}`
    : `signal ${context.signalCode}${context.exitCode == null ? "" : `, exit code ${context.exitCode}`}`;
  const stderr = context.stderr.trim();
  return new Error([
    `${context.operation}: browser process exited before reporting (${termination})`,
    `binary: ${context.binary}`,
    `page: ${context.pageUrl}`,
    ...(stderr ? [`browser stderr (tail):\n${stderr}`] : ["browser stderr: <empty>"]),
  ].join("\n"));
}

export async function readBrowserStderrTail(path: string, maximumBytes = 8 * 1024): Promise<string> {
  try {
    const file = Bun.file(path);
    if (!(await file.exists())) return "";
    const start = Math.max(0, file.size - maximumBytes);
    return await file.slice(start, file.size).text();
  } catch {
    return "";
  }
}

function browserReportAbortError(): Error {
  const error = new Error("browser report polling aborted");
  error.name = "AbortError";
  return error;
}

function waitForBrowserReportRetry(signal: AbortSignal, delayMs: number): Promise<void> {
  if (signal.aborted) return Promise.reject(browserReportAbortError());
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, delayMs);
    const onAbort = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", onAbort);
      reject(browserReportAbortError());
    };
    signal.addEventListener("abort", onAbort, { once: true });
    if (signal.aborted) onAbort();
  });
}

/** Poll a Valle-hosted browser report until it exists or the caller cancels the wait. */
export async function pollBrowserReport(
  reportUrl: string,
  signal: AbortSignal,
  retryDelayMs = 250,
): Promise<any> {
  for (;;) {
    if (signal.aborted) throw browserReportAbortError();
    let response: Response | undefined;
    try {
      response = await fetch(reportUrl, { signal });
    } catch {
      if (signal.aborted) throw browserReportAbortError();
    }
    if (response?.ok) return await response.json();
    await waitForBrowserReportRetry(signal, retryDelayMs);
  }
}

export interface BrowserExitWatchContext {
  operation: string;
  binary: string;
  pageUrl: string;
  stderrPath: string;
  isCompleted: () => boolean;
}

export async function waitForUnexpectedBrowserExit(
  child: Bun.Subprocess,
  context: BrowserExitWatchContext,
): Promise<never> {
  const exitCode = await child.exited;
  if (context.isCompleted()) return await new Promise<never>(() => undefined);
  throw browserExitError({
    operation: context.operation,
    binary: context.binary,
    pageUrl: context.pageUrl,
    exitCode,
    signalCode: child.signalCode,
    stderr: await readBrowserStderrTail(context.stderrPath),
  });
}

export type BrowserTermination = "already-exited" | "terminated" | "killed";

/** Let a healthy Chrome persist its profile and exit normally; reserve SIGKILL for a hung child. */
export async function terminateBrowser(
  child: Bun.Subprocess,
  gracefulTimeoutMs = 5_000,
): Promise<BrowserTermination> {
  if (child.exitCode != null) {
    await child.exited.catch(() => undefined);
    return "already-exited";
  }

  // Bun's signal-less `kill()` sends SIGTERM. Chromium handles it as an orderly shutdown, unlike
  // the previous unconditional signal 9 which can trigger an OS "Chrome quit unexpectedly" alert.
  try {
    child.kill();
  } catch {
    // The process may have exited between the `exitCode` observation and this signal.
  }

  let timeout: ReturnType<typeof setTimeout> | undefined;
  const exited = child.exited.then(
    () => true,
    () => true,
  );
  const timedOut = new Promise<false>((resolve) => {
    timeout = setTimeout(() => resolve(false), gracefulTimeoutMs);
  });
  const graceful = await Promise.race([exited, timedOut]);
  if (timeout !== undefined) clearTimeout(timeout);
  if (graceful) return "terminated";

  if (child.exitCode == null) {
    try {
      child.kill(9);
    } catch {
      // A concurrent normal exit is also a successful cleanup.
    }
  }
  await child.exited.catch(() => undefined);
  return "killed";
}

export async function decodeVideoFramesInBrowser(
  browser: string,
  videoPath: string,
  timesS: readonly number[],
  options: BrowserDecodeOptions = {},
): Promise<BrowserDecodeResult> {
  const timeoutMs = options.timeoutMs ?? 45_000;
  const [decodePage, playerCore, video] = await Promise.all([
    Bun.file(DECODE_PAGE).bytes(),
    Bun.file(PLAYER_CORE_JS).bytes(),
    Bun.file(videoPath).bytes(),
  ]);

  let resolveResult!: (value: BrowserDecodeWireResult) => void;
  let rejectResult!: (reason: unknown) => void;
  const resultPromise = new Promise<BrowserDecodeWireResult>((resolve, reject) => {
    resolveResult = resolve;
    rejectResult = reject;
  });
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 40_000 + Math.floor(Math.random() * 20_000),
    async fetch(request) {
      const url = new URL(request.url);
      if (request.method === "POST" && url.pathname === "/result") {
        try {
          resolveResult(await request.json() as BrowserDecodeWireResult);
          return new Response(null, { status: 204 });
        } catch (error) {
          rejectResult(error);
          return new Response("invalid report", { status: 400 });
        }
      }
      if (url.pathname === "/decode.html") return new Response(decodePage, { headers: { "content-type": "text/html" } });
      if (url.pathname === "/packages/player-core/index.mjs") return new Response(playerCore, { headers: { "content-type": "text/javascript" } });
      if (url.pathname === "/video.mp4") return new Response(video, { headers: { "content-type": "video/mp4" } });
      return new Response("not found", { status: 404 });
    },
  });
  const pageUrl = `http://127.0.0.1:${server.port}/decode.html?src=/video.mp4&times=${timesS.join(",")}`;
  const profileDir = createTempDirectory("valle-webcodecs");
  const stderrPath = `${profileDir}/chrome.stderr.log`;
  const child = Bun.spawn({
    cmd: [
      browser,
      "--headless=new",
      "--no-sandbox",
      `--user-data-dir=${profileDir}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-extensions",
      "--mute-audio",
      "--hide-scrollbars",
      pageUrl,
    ],
    stdin: "ignore",
    stdout: "ignore",
    stderr: Bun.file(stderrPath),
  });
  const timer = setTimeout(() => rejectResult(new Error(`browser decode timed out after ${timeoutMs}ms`)), timeoutMs);
  let completed = false;
  const browserExit = waitForUnexpectedBrowserExit(child, {
    operation: "browser decode",
    binary: browser,
    pageUrl,
    stderrPath,
    isCompleted: () => completed,
  });

  try {
    const result = await Promise.race([resultPromise, browserExit]);
    completed = true;
    if (result.status !== "ok") throw new Error(`browser decode failed: ${result.message ?? JSON.stringify(result)}`);
    if (!result.frames || result.frames.length !== timesS.length) {
      throw new Error(`browser returned ${result.frames?.length ?? 0} frames for ${timesS.length} times`);
    }
    const frames = result.frames.map(({ rgbaBase64, ...frame }): BrowserDecodedFrame => {
      const data = Uint8Array.fromBase64(rgbaBase64);
      const expected = frame.width * frame.height * 4;
      if (data.length !== expected) throw new Error(`frame at t=${frame.timeS}: got ${data.length} bytes, want ${expected}`);
      return { ...frame, data };
    });
    return { implementation: result.implementation, userAgent: result.userAgent, frames };
  } finally {
    completed = true;
    clearTimeout(timer);
    server.stop(true);
    await terminateBrowser(child);
    removeDirectory(profileDir);
  }
}
