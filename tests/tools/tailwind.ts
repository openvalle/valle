// Record official compiler output for review; never overwrite accepted baselines.
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { __unstable__loadDesignSystem } from "tailwindcss";

const package_ = join(dirname(Bun.resolveSync("tailwindcss/package.json", import.meta.dir)), "/");
export const version: string = (await Bun.file(join(package_, "package.json")).json()).version;

// The theme and the utilities, without the preflight, which the engine does not have.
const INPUT = '@import "tailwindcss/theme.css";\n@import "tailwindcss/utilities.css";\n';

const options = {
  base: package_,
  async loadStylesheet(id: string) {
    const path = join(package_, `${id.replace(/^tailwindcss\/?/, "")}`);
    return { path, base: package_, content: await readFile(path, "utf8") };
  },
};

type Node =
  | { kind: "rule"; selector: string; nodes: Node[] }
  | { kind: "at-rule"; name: string; params: string; nodes: Node[] }
  | { kind: "declaration"; property: string; value?: string; important: boolean }
  | { kind: string; nodes?: Node[] };

/** A class's CSS on one line, its own selector `&`, without the declarations of no value, which
 * Tailwind leaves out of its stylesheets. */
function print(nodes: Node[], selector: string): string {
  const all = (nodes: Node[]) => nodes.map(one).filter(Boolean).join(" ");
  const one = (n: Node): string => {
    if (n.kind === "declaration" && "property" in n) {
      if (n.value === undefined) return "";
      return `${n.property}: ${n.value}${n.important ? " !important" : ""};`;
    }
    if (n.kind === "rule" && "selector" in n) {
      return `${n.selector.split(selector).join("&")} { ${all(n.nodes)} }`;
    }
    if (n.kind === "at-rule" && "name" in n) {
      const body = n.nodes.length > 0 ? ` { ${all(n.nodes)} }` : ";";
      return `${n.name} ${n.params}${body}`;
    }
    throw new Error(`a ${n.kind} node in a class's CSS`);
  };
  return all(nodes);
}

/** `CSS.escape`, as Tailwind escapes a class into its selector. */
function escape(s: string): string {
  if (s === "-") return "\\-";
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    const digit = c >= 0x30 && c <= 0x39;
    if (c === 0) {
      out += "�";
    } else if (
      (c >= 0x01 && c <= 0x1f) ||
      c === 0x7f ||
      (i === 0 && digit) ||
      (i === 1 && digit && s.charCodeAt(0) === 0x2d)
    ) {
      out += `\\${c.toString(16)} `;
    } else if (c >= 0x80 || c === 0x2d || c === 0x5f || digit || /[A-Za-z]/.test(s[i])) {
      out += s[i];
    } else {
      out += `\\${s[i]}`;
    }
  }
  return out;
}

async function main() {
  const repo = join(import.meta.dir, "../..");
  const request = join(repo, "tests/cases/tailwind/classes.json");
  const out = join(repo, "target/baseline-candidates/tailwind/compiled.json");
  const asked: string[] = (await Bun.file(request).json()).classes;
  const system = await __unstable__loadDesignSystem(INPUT, options);
  const list: string[] = system.getClassList().map(([name]: [string]) => name);
  const classes = [...new Set([...list, ...asked])];
  const properties: Record<string, string | null> = {};
  const css = new Map<string, string | null>();
  for (const [i, nodes] of system.candidatesToAst(classes).entries()) {
    const own = [];
    for (const n of nodes as Node[]) {
      if (n.kind === "at-rule" && "name" in n && n.name === "@property") {
        const initial = n.nodes.find((d) => "property" in d && d.property === "initial-value");
        properties[n.params] = initial && "value" in initial ? (initial.value ?? null) : null;
      } else {
        own.push(n);
      }
    }
    css.set(classes[i], own.length > 0 ? print(own, `.${escape(classes[i])}`) : null);
  }
  // Tailwind's order; `getClassOrder` gives none to classes it makes nothing of.
  const order = new Map<string, bigint | null>(system.getClassOrder(classes));
  const made = classes.filter((c) => css.get(c) !== null);
  made.sort((a, z) => {
    const [x, y] = [order.get(a)!, order.get(z)!];
    return x < y ? -1 : x > y ? 1 : 0;
  });
  const sorted = [...made, ...classes.filter((c) => css.get(c) === null).sort()];
  const sortedProperties = Object.fromEntries(
    Object.entries(properties).sort(([a], [z]) => (a < z ? -1 : 1)),
  );
  await Bun.write(
    out,
    JSON.stringify({
      tailwind: version,
      list,
      properties: sortedProperties,
      classes: sorted.map((c) => [c, css.get(c)]),
    }),
  );
}

await main();
console.log("Candidate official output: target/baseline-candidates/tailwind/compiled.json");
