import { expect, test } from "bun:test";
import { moduleClosure } from "./studio-compile.ts";

test("Studio source closure uses Host canonical paths for a symlinked entry", () => {
  expect(moduleClosure("/tmp/project/card.motion.tsx", {
    "/private/tmp/project/card.motion.tsx": "import './parts/title';",
    "/private/tmp/project/parts/title.ts": "export const title = 'hello';",
  })).toEqual({
    entry: "card.motion.tsx",
    modules: {
      "card.motion.tsx": "import './parts/title';",
      "parts/title.ts": "export const title = 'hello';",
    },
  });
});
