// oxlint-disable-next-line no-control-regex -- Non-ASCII text needs the conservative font closure.
const nonAscii = /[^\x00-\x7f]/u;

/** Determine the conservative font closure after compilation, before downloading the pack.
 * Unknown text or family choices keep every face; ASCII-only scenes avoid the 30 MB CJK face.
 * Measurement/outline helpers are handled by the worker before this rendering-only selection. */
export function artifactFontUrls<T extends { url: string; role: "font" | "formula-font" }>(
  artifact: Record<string, unknown>,
  urls: readonly T[],
  boundFontCoversText?: (family: string, text: string) => boolean,
): T[] {
  let text = false,
    formula = false,
    all = false,
    serif = false,
    mono = false,
    boundTextOnly = true;
  const boundText = new Map<string, Set<string>>();
  const familyValue = (value: unknown): unknown => {
    if (value && typeof value === "object") {
      const token = value as { kind?: string; value?: unknown };
      if (token.kind === "str" || token.kind === "enum") return token.value;
    }
    return value;
  };
  const visit = (value: unknown): void => {
    if (!value || typeof value !== "object") return;
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    const object = value as Record<string, unknown>;
    const kind = object.kind as Record<string, unknown> | undefined;
    if (kind?.kind === "text") {
      text = true;
      const content = kind.text as { kind?: string; value?: string } | undefined;
      if (
        content?.kind !== "static" ||
        typeof content.value !== "string" ||
        nonAscii.test(content.value)
      )
        all = true;
      const styles = (object.styles ?? []) as Array<{
        property: string;
        value?: { kind?: string; value?: unknown };
      }>;
      const families = styles.filter((style) => style.property === "font-family");
      const family = familyValue(families[0]?.value?.value);
      if (
        content?.kind === "static" &&
        typeof content.value === "string" &&
        families.length === 1 &&
        families[0]?.value?.kind === "static" &&
        typeof family === "string" &&
        /^valle-font-[0-9a-f]{64}$/u.test(family) &&
        !styles.some((style) => style.property === "font") &&
        !((object.classNames ?? []) as string[]).some((name) => name.includes("font"))
      ) {
        let characters = boundText.get(family);
        if (!characters) boundText.set(family, (characters = new Set()));
        for (const character of content.value) characters.add(character);
      } else boundTextOnly = false;
    }
    if (kind?.kind === "mathFormula") formula = true;
    for (const style of (object.styles ?? []) as Array<{
      property: string;
      value?: { kind?: string; value?: unknown };
    }>) {
      if (style.property === "font-family") {
        const family = familyValue(style.value?.value);
        if (
          style.value?.kind !== "static" ||
          typeof family !== "string" ||
          !/^(sans-serif|serif|monospace|ui-sans-serif|ui-serif|ui-monospace)$/u.test(family)
        )
          all = true;
        serif ||= typeof family === "string" && /^(ui-)?serif$/u.test(family);
        mono ||= typeof family === "string" && /mono/u.test(family);
      } else if (style.property === "font" || style.property.startsWith("--font-")) all = true;
    }
    for (const tokens of (object.classNames ?? []) as string[])
      for (const token of tokens.split(/\s+/u)) {
        const name = token.split(":").at(-1)!.replace(/!$/u, "");
        serif ||= name === "font-serif";
        mono ||= name === "font-mono";
        if (
          name.includes("font") &&
          !/^font-(sans|serif|mono|thin|extralight|light|normal|medium|semibold|bold|extrabold|black)$/u.test(
            name,
          )
        )
          all = true;
      }
    // Includes instance templates and nested template children, whose text isn't in nodes[].
    for (const key of [
      "nodes",
      "instanceGroups",
      "template",
      "templateChildren",
      "node",
      "children",
    ])
      visit(object[key]);
  };
  visit(artifact);
  const covered =
    text &&
    boundTextOnly &&
    boundFontCoversText !== undefined &&
    [...boundText].every(([family, characters]) =>
      boundFontCoversText(family, [...characters].join("")),
    );
  return urls.filter(({ url, role }) => {
    if (role === "formula-font") return formula;
    if (!text || covered) return false;
    if (all || !url.startsWith("/runtime/fonts/")) return true;
    const file = url.split("/").at(-1)!;
    return (
      file === "NotoSans-Variable.ttf" ||
      (serif && file.startsWith("KaTeX_Main-")) ||
      (mono && file === "NotoSansMono-Regular.ttf")
    );
  });
}
