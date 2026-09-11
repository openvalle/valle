// Run with bun web/tests/parity/cli.ts browser.
//
// Usage: bun web/tests/parity/cli.ts browser --request <request.json>. The request selects a built runtime page; preview.html is the default.
import type { CommandDefinition } from "../lib/args.ts";
import { requiredOption } from "../lib/args.ts";
import { createTempDirectory, directoryName, ensureDirectory, joinPath, readText, relativePath, removeDirectory, repositoryPath, resolveRequestPath, resolveUnder, writeBytes, } from "../lib/files.ts";
import { readPng } from "../lib/png.ts";
import { comparePngs, countNonBlackPixels } from "../lib/metrics.ts";
import { pollBrowserReport, resolveBrowser, terminateBrowser, waitForUnexpectedBrowserExit } from "../lib/browser.ts";
const WEB_DIST_DIR = repositoryPath("web/dist");
const RUNTIME_MANIFEST = JSON.parse(await readText(joinPath(WEB_DIST_DIR, "runtime/manifest.json")));
const APP_HTML = new Map(RUNTIME_MANIFEST.apps.map(({ id, html }: any) => [`${id}.html`, joinPath(WEB_DIST_DIR, html)]));
export const browserCommand: CommandDefinition = {
    name: "browser",
    summary: "Run Demo, Studio or Console parity in a real Chromium browser.",
    options: [
        { name: "request", value: "request.json", description: "browser smoke request", required: true },
    ],
    async run(options: any) {
        await runBrowser(requiredOption(options, "request"));
    },
};
async function runBrowser(requestPath: any) {
    const requestDir = directoryName(requestPath);
    const request = JSON.parse(await readText(requestPath));
    if (request.protocolVersion !== 1) {
        throw new Error(`unsupported protocolVersion ${request.protocolVersion}`);
    }
    const browser = resolveBrowser();
    if (!browser) {
        if (request.requireBrowser)
            throw new Error("browser unavailable");
        process.stdout.write(`${JSON.stringify({ status: "skipped", reason: "browser_unavailable" }, null, 2)}\n`);
        return;
    }
    const page = request.page ?? "preview.html";
    if (!["preview.html", "studio.html", "console.html"].includes(page)) {
        throw new Error(`unsupported page '${page}' (preview.html | studio.html | console.html)`);
    }
    // With serverUrl, use the running Valle host and poll its smoke-report endpoint.
    const serverUrl = request.serverUrl ? String(request.serverUrl).replace(/\/$/, "") : null;
    let server: any = null;
    let resolveResult: (value: any) => void = () => undefined;
    let rejectResult: (reason: unknown) => void = () => undefined;
    let cancelResultWait: () => void = () => undefined;
    let resultPromise: Promise<any>;
    let pageUrl: string;
    if (serverUrl) {
        // Append page parameters such as the Studio project ID.
        const extraQuery = request.pageQuery ? `&${request.pageQuery}` : "";
        pageUrl = `${serverUrl}/${page}?smoke=1${extraQuery}`;
        const pollAbort = new AbortController();
        cancelResultWait = () => pollAbort.abort();
        const poll = pollBrowserReport(`${serverUrl}/smoke-report`, pollAbort.signal);
        // Retain a rejection channel to interrupt polling after timeouts or browser startup failures.
        resultPromise = Promise.race([poll, new Promise((_: any, reject: any) => {
                rejectResult = reject;
            })]);
    }
    else {
        const timelineJson = request.timelineJson ?? JSON.stringify(request.timeline);
        const config = {
            timelineJson,
            timeline: request.timeline ?? JSON.parse(timelineJson),
            assets: request.assets ?? [],
            components: request.components ?? {},
            motion: request.motion ?? null,
            captures: request.captures ?? [],
            initialTimeS: request.initialTimeS ?? request.captures?.[0]?.timeS ?? 0,
            playMs: request.playMs ?? 180,
            fps: request.timeline?.canvas?.fps,
            runtimeAssets: RUNTIME_MANIFEST.runtimeAssets,
            runtimeBaseUrl: "/",
        };
        const assetsDir = request.assetsDir ? resolveRequestPath(requestDir, request.assetsDir) : null;
        resultPromise = new Promise((resolve: any, reject: any) => {
            resolveResult = resolve;
            rejectResult = reject;
        });
        server = Bun.serve({
            hostname: "127.0.0.1",
            port: 40000 + Math.floor(Math.random() * 20000),
            fetch: (incoming: any) => handleRequest(incoming, {
                config,
                requestDir,
                assetsDir,
                resolveResult,
                rejectResult,
            }),
        });
        const extraQuery = request.pageQuery ? `&${request.pageQuery}` : "";
        pageUrl = `http://127.0.0.1:${server.port}/${page}?smoke=1${extraQuery}`;
    }
    const profileDir = createTempDirectory("valle-web-player");
    const stderrPath = joinPath(profileDir, "chrome.stderr.log");
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
            "--autoplay-policy=no-user-gesture-required",
            pageUrl,
        ],
        stdin: "ignore",
        // The report travels over HTTP. Chromium descendants inherit pipe descriptors, so waiting
        // for piped stdout/stderr after killing only the browser parent can block forever.
        stdout: "ignore",
        // A regular file preserves startup/crash diagnostics without the inherited-pipe hang.
        stderr: Bun.file(stderrPath),
    });
    const timer = setTimeout(() => rejectResult(new Error(`browser smoke timed out after ${request.timeoutMs ?? 45000}ms`)), request.timeoutMs ?? 45000);
    let completed = false;
    const browserExit = waitForUnexpectedBrowserExit(child, {
        operation: "browser smoke",
        binary: browser,
        pageUrl,
        stderrPath,
        isCompleted: () => completed,
    });
    try {
        const browserReport = await Promise.race([resultPromise, browserExit]);
        completed = true;
        if (browserReport.status !== "ok") {
            throw new Error(`browser smoke failed: ${browserReport.message ?? JSON.stringify(browserReport)}`);
        }
        const report = await materializeCaptures(request, requestDir, browserReport);
        console.log(JSON.stringify(report, null, 2));
    }
    finally {
        completed = true;
        clearTimeout(timer);
        cancelResultWait();
        server?.stop(true);
        await terminateBrowser(child);
        removeDirectory(profileDir);
    }
}
async function handleRequest(request: any, state: any) {
    const url = new URL(request.url);
    if (request.method === "POST" && url.pathname === "/result") {
        try {
            state.resolveResult(await request.json());
            return new Response(null, { status: 204 });
        }
        catch (error) {
            state.rejectResult(error);
            return new Response("invalid report", { status: 400 });
        }
    }
    try {
        if (url.pathname === "/preview.html" || url.pathname === "/") {
            return serveFile(APP_HTML.get("preview.html"), "text/html");
        }
        else if (url.pathname === "/studio.html" || url.pathname === "/console.html") {
            return serveFile(APP_HTML.get(url.pathname.slice(1)), "text/html");
        }
        else if (url.pathname === "/studio/boot.json") {
            return Response.json({
                protocolVersion: 1,
                session: { kind: "timeline-file", input: "parity-timeline" },
                capabilities: {
                    saveTimeline: false,
                    editProject: false,
                    editMotionProps: false,
                    writeMotionSource: false,
                },
                runtime: {
                    assetBaseUrl: "/assets/",
                    assetUrls: {
                        engineGlue: RUNTIME_MANIFEST.runtimeAssets.engine.glue,
                        engineWasm: RUNTIME_MANIFEST.runtimeAssets.engine.wasm,
                        canvasKitFullGlue: RUNTIME_MANIFEST.runtimeAssets.canvasKit.full.glue,
                        canvasKitFullWasm: RUNTIME_MANIFEST.runtimeAssets.canvasKit.full.wasm,
                    },
                },
            });
        }
        else if (url.pathname === "/config.json") {
            return Response.json(state.config);
        }
        else if (url.pathname.startsWith("/chunks/") || url.pathname.startsWith("/runtime/")) {
            const file = resolveUnder(WEB_DIST_DIR, url.pathname.slice(1));
            return serveFile(file, contentTypeFor(file));
        }
        else if (url.pathname.startsWith("/assets/")) {
            if (!state.assetsDir)
                throw new Error("request has no assetsDir");
            const rel = decodeURIComponent(url.pathname.slice("/assets/".length));
            const file = resolveUnder(state.assetsDir, rel);
            return serveFile(file, contentTypeFor(file));
        }
        return new Response("not found", { status: 404 });
    }
    catch (err) {
        return new Response(err instanceof Error ? err.stack ?? err.message : String(err), { status: 500, headers: { "content-type": "text/plain" } });
    }
}
async function materializeCaptures(request: any, requestDir: any, browserReport: any) {
    const captures = [];
    for (const capture of browserReport.captures ?? []) {
        const requestCapture = (request.captures ?? []).find((c: any) => c.sampleId === capture.sampleId);
        if (!requestCapture)
            throw new Error(`browser returned unexpected capture '${capture.sampleId}'`);
        const data = Uint8Array.fromBase64(capture.pngBase64);
        const webPngPath = resolveRequestPath(requestDir, requestCapture.webPng ?? `${capture.sampleId}.web.png`);
        ensureDirectory(directoryName(webPngPath));
        await writeBytes(webPngPath, data);
        const web = await readPng(webPngPath);
        const nativePngPath = resolveRequestPath(requestDir, requireString(requestCapture.nativePng, "nativePng"));
        const metrics = comparePngs(await readPng(nativePngPath), web);
        captures.push({
            ...capture,
            pngBase64: undefined,
            output: relativePath(requestDir, webPngPath),
            nonBlackPixels: countNonBlackPixels(web.data),
            metrics,
        });
    }
    return {
        status: "ok",
        protocolVersion: request.protocolVersion,
        caseId: request.caseId,
        implementation: browserReport.implementation,
        userAgent: browserReport.userAgent,
        stats: browserReport.stats,
        captures,
    };
}
async function serveFile(file: any, contentType: any) {
    const body = Bun.file(file);
    if (!(await body.exists()))
        return new Response("not found", { status: 404 });
    return new Response(body, { headers: { "content-type": contentType } });
}
function contentTypeFor(file: any) {
    if (file.endsWith(".html"))
        return "text/html";
    if (file.endsWith(".js"))
        return "text/javascript";
    if (file.endsWith(".wasm"))
        return "application/wasm";
    if (file.endsWith(".ttf"))
        return "font/ttf";
    if (file.endsWith(".json"))
        return "application/json";
    if (file.endsWith(".png"))
        return "image/png";
    if (file.endsWith(".jpg") || file.endsWith(".jpeg"))
        return "image/jpeg";
    if (file.endsWith(".mp4"))
        return "video/mp4";
    return "application/octet-stream";
}
function requireString(value: any, name: any) {
    if (typeof value !== "string" || value.length === 0)
        throw new Error(`${name} must be a non-empty string`);
    return value;
}
