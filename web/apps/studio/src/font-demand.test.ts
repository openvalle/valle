import { expect, test } from "bun:test";
import { artifactFontUrls } from "./font-demand.ts";
const urls = ["NotoSans-Variable.ttf", "NotoSansCJKsc-Variable.otf", "Noto-COLRv1.ttf", "KaTeX_Main-Regular.ttf"]
  .map(name => ({ url: `/runtime/fonts/${name}`, role: "font" as const }));
const text = (value: string) => ({kind:{kind:"text",text:{kind:"static",value}}});
test("font closure includes nested instance text and loads CJK only when needed", () => {
  expect(artifactFontUrls({nodes:[]},urls)).toEqual([]);
  const scene = { nodes:[], instanceGroups:[{template:{kind:{kind:"box"}},templateChildren:[{node:text("Hello")}]}] };
  expect(artifactFontUrls(scene,urls)).toEqual(urls.slice(0,1));
  expect(artifactFontUrls({nodes:[text("中文")]},urls)).toEqual(urls);
  expect(artifactFontUrls({nodes:[{kind:{kind:"text",text:{kind:"expr",expr:1}}}]},urls)).toEqual(urls);
});
