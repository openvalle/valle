import { expect, test } from "bun:test";
import { artifactFontUrls } from "./font-demand.ts";
const urls = [
  "NotoSans-Variable.ttf",
  "NotoSansCJKsc-Variable.otf",
  "Noto-COLRv1.ttf",
  "KaTeX_Main-Regular.ttf",
].map((name) => ({ url: `/runtime/fonts/${name}`, role: "font" as const }));
const text = (value: string) => ({ kind: { kind: "text", text: { kind: "static", value } } });
test("font closure includes nested instance text and loads CJK only when needed", () => {
  expect(artifactFontUrls({ nodes: [] }, urls)).toEqual([]);
  const scene = {
    nodes: [],
    instanceGroups: [
      { template: { kind: { kind: "box" } }, templateChildren: [{ node: text("Hello") }] },
    ],
  };
  expect(artifactFontUrls(scene, urls)).toEqual(urls.slice(0, 1));
  expect(artifactFontUrls({ nodes: [text("中文")] }, urls)).toEqual(urls);
  expect(
    artifactFontUrls(
      { nodes: [{ kind: { kind: "text", text: { kind: "expr", expr: 1 } } }] },
      urls,
    ),
  ).toEqual(urls);
});

test("bound text skips default faces only after its explicit face proves glyph coverage", () => {
  const family = `valle-font-${"a".repeat(64)}`;
  const node = {
    ...text("中文 Valle"),
    styles: [
      {
        property: "font-family",
        value: { kind: "static", value: { kind: "str", value: family } },
      },
    ],
  };
  const artifact = { nodes: [node] };
  const checked: string[] = [];
  expect(
    artifactFontUrls(artifact, urls, (name, value) => {
      expect(name).toBe(family);
      checked.push(value);
      return true;
    }),
  ).toEqual([]);
  expect(checked).toEqual(["中文 Vale"]);
  expect(artifactFontUrls(artifact, urls, () => false)).toEqual(urls);
  expect(artifactFontUrls(artifact, urls)).toEqual(urls);
  expect(artifactFontUrls({ nodes: [node, text("unbound")] }, urls, () => true)).toEqual(urls);
  expect(
    artifactFontUrls(
      { nodes: [{ ...node, kind: { kind: "text", text: { kind: "expr", expr: 0 } } }] },
      urls,
      () => true,
    ),
  ).toEqual(urls);
});

test("compiled MotionValue font families retain the requested generic faces", () => {
  expect(
    artifactFontUrls(
      {
        nodes: [
          {
            ...text("Hello"),
            styles: [
              {
                property: "font-family",
                value: { kind: "static", value: { kind: "str", value: "serif" } },
              },
            ],
          },
        ],
      },
      urls,
    ),
  ).toEqual([urls[0]!, urls[3]!]);
});
