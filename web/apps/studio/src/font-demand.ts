/** Determine the conservative font closure after compilation, before downloading the pack.
 * Unknown text or family choices keep every face; ASCII-only scenes avoid the 30 MB CJK face.
 * Measurement/outline helpers are handled by the worker before this rendering-only selection. */
export function artifactFontUrls<T extends { url: string; role: "font" | "formula-font" }>(
  artifact: Record<string, unknown>, urls: readonly T[],
): T[] {
  let text = false, formula = false, all = false, serif = false, mono = false;
  const visit = (value: unknown): void => {
    if (!value || typeof value !== "object") return;
    if (Array.isArray(value)) { value.forEach(visit); return; }
    const object = value as Record<string, unknown>;
    const kind = object.kind as Record<string, unknown> | undefined;
    if (kind?.kind === "text") {
      text = true;
      const content = kind.text as { kind?: string; value?: string } | undefined;
      if (content?.kind !== "static" || typeof content.value !== "string" || /[^\x00-\x7f]/u.test(content.value)) all = true;
    }
    if (kind?.kind === "mathFormula") formula = true;
    for (const style of (object.styles ?? []) as Array<{ property: string; value?: { kind?: string; value?: unknown } }>) {
      if (style.property === "font-family") {
        const family = style.value?.value;
        if (style.value?.kind !== "static" || typeof family !== "string"
          || !/^(sans-serif|serif|monospace|ui-sans-serif|ui-serif|ui-monospace)$/u.test(family)) all = true;
        serif ||= typeof family === "string" && /^(ui-)?serif$/u.test(family);
        mono ||= typeof family === "string" && /mono/u.test(family);
      } else if (style.property === "font" || style.property.startsWith("--font-")) all = true;
    }
    for (const tokens of (object.classNames ?? []) as string[]) for (const token of tokens.split(/\s+/u)) {
      const name = token.split(":").at(-1)!.replace(/!$/u, "");
      serif ||= name === "font-serif";
      mono ||= name === "font-mono";
      if (name.includes("font") && !/^font-(sans|serif|mono|thin|extralight|light|normal|medium|semibold|bold|extrabold|black)$/u.test(name)) all = true;
    }
    // Includes instance templates and nested template children, whose text isn't in nodes[].
    for (const key of ["nodes", "instanceGroups", "template", "templateChildren", "node", "children"]) visit(object[key]);
  };
  visit(artifact);
  return urls.filter(({ url, role }) => {
    if (role === "formula-font") return formula;
    if (!text) return false;
    if (all || !url.startsWith("/runtime/fonts/")) return true;
    const file = url.split("/").at(-1)!;
    return file === "NotoSans-Variable.ttf" || (serif && file.startsWith("KaTeX_Main-"))
      || (mono && file === "NotoSansMono-Regular.ttf");
  });
}
