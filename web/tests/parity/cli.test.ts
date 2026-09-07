import { describe, expect, test } from "bun:test";

const bun = Bun.which("bun");
if (!bun) throw new Error("bun executable is unavailable");
const cli = Bun.fileURLToPath(new URL("./cli.ts", import.meta.url));

function run(...args: string[]) {
  const result = Bun.spawnSync({ cmd: [bun!, cli, ...args], stdout: "pipe", stderr: "pipe" });
  return {
    exitCode: result.exitCode,
    stdout: new TextDecoder().decode(result.stdout),
    stderr: new TextDecoder().decode(result.stderr),
  };
}

describe("parity CLI", () => {
  test("lists typed commands", () => {
    const result = run("--help");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain("math");
    expect(result.stdout).toContain("browser");
    expect(result.stderr).toBe("");
  });

  test("reports command help without running it", () => {
    const result = run("browser", "--help");
    expect(result.exitCode).toBe(0);
    expect(result.stdout).toContain("--request <request.json>");
  });

  test("uses exit code 2 for contract errors", () => {
    const missing = run("math", "--wasm", "probe.wasm");
    expect(missing.exitCode).toBe(2);
    expect(missing.stderr).toContain("missing required option: --golden");

    const unknown = run("not-a-command");
    expect(unknown.exitCode).toBe(2);
    expect(unknown.stderr).toContain("unknown parity command");
  });
});
