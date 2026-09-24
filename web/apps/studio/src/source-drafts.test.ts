import { expect, test } from "bun:test";
import { SourceDrafts } from "./source-drafts.ts";
import type { StudioHost, SourceFile } from "./host.ts";

const loaded: SourceFile[] = [
  { path: "/a.motion.tsx", status: "ok", text: "A", baseDigest: "sha256:a" },
  { path: "/b.ts", status: "ok", text: "B", baseDigest: "sha256:b" },
];

test("source saves keep successful files and retain failed drafts", async () => {
  const drafts = new SourceDrafts(loaded);
  drafts.edit("/a.motion.tsx", "A2");
  drafts.edit("/b.ts", "B2");
  const calls: string[] = [];
  const host = {
    writeSourceFile: async ({ path }: { path: string }) => {
      calls.push(path);
      return path === "/a.motion.tsx"
        ? { status: "saved" as const, digest: "sha256:a2" }
        : { status: "conflict" as const, currentDigest: "sha256:external" };
    },
  } as unknown as StudioHost;
  const result = await drafts.saveDirty(host);
  expect(calls).toEqual(["/a.motion.tsx", "/b.ts"]);
  expect(result.saved).toEqual({ "/a.motion.tsx": "sha256:a2" });
  expect(result.failed?.path).toBe("/b.ts");
  expect(drafts.files.get("/a.motion.tsx")?.savedText).toBe("A2");
  expect(drafts.files.get("/b.ts")?.text).toBe("B2");
  expect(drafts.dirty).toBe(true);
});

test("edits made during a save remain dirty after the captured bytes are written", async () => {
  const drafts = new SourceDrafts(loaded);
  drafts.edit("/a.motion.tsx", "A2");
  const host = {
    writeSourceFile: async () => {
      drafts.edit("/a.motion.tsx", "A3");
      return { status: "saved" as const, digest: "sha256:a2" };
    },
  } as unknown as StudioHost;
  expect((await drafts.saveDirty(host)).failed).toBeNull();
  expect(drafts.files.get("/a.motion.tsx")?.savedText).toBe("A2");
  expect(drafts.files.get("/a.motion.tsx")?.text).toBe("A3");
  expect(drafts.dirty).toBe(true);
});

test("external source changes refresh clean files and preserve dirty drafts", () => {
  const drafts = new SourceDrafts(loaded);
  drafts.edit("/b.ts", "B local");
  drafts.reconcile([
    { path: "/a.motion.tsx", status: "ok", text: "A external", baseDigest: "sha256:a-external" },
    { path: "/b.ts", status: "ok", text: "B external", baseDigest: "sha256:b-external" },
    { path: "/new.ts", status: "ok", text: "new", baseDigest: "sha256:new" },
  ]);
  expect(drafts.files.get("/a.motion.tsx")?.text).toBe("A external");
  expect(drafts.files.get("/a.motion.tsx")?.baseDigest).toBe("sha256:a-external");
  expect(drafts.files.get("/b.ts")?.text).toBe("B local");
  expect(drafts.files.get("/b.ts")?.baseDigest).toBe("sha256:b");
  expect(drafts.files.get("/b.ts")?.conflict).toBe("Source changed outside Studio");
  expect(drafts.files.get("/new.ts")?.text).toBe("new");
});

test("matching disk text confirms a write with a missing response", () => {
  const drafts = new SourceDrafts(loaded);
  drafts.edit("/a.motion.tsx", "A2");
  drafts.reconcile([
    { path: "/a.motion.tsx", status: "ok", text: "A2", baseDigest: "sha256:a2" },
  ]);
  expect(drafts.dirty).toBe(false);
  expect(drafts.files.get("/a.motion.tsx")?.baseDigest).toBe("sha256:a2");
});

test("unchanged import scans do not schedule another preview", () => {
  const drafts = new SourceDrafts(loaded);
  drafts.edit("/a.motion.tsx", "A local");
  expect(drafts.reconcile(loaded)).toBe(false);
  expect(drafts.reconcile([...loaded,
    { path: "/new.ts", status: "ok", text: "new", baseDigest: "sha256:new" },
  ])).toBe(true);
  expect(drafts.reconcile([...loaded,
    { path: "/new.ts", status: "ok", text: "new", baseDigest: "sha256:new" },
  ])).toBe(false);
});
