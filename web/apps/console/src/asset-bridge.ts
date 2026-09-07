export type ProjectAssetLibraryBridge =
  | { status: "matched"; contentDigest: string }
  | { status: "ambiguous"; contentDigests: string[] }
  | { status: "missing" }
  | { status: "unaddressable" };

const PROJECT_ASSET_ID = /^asset_([0-9a-f]{16})$/;
const LIBRARY_CONTENT_HEX = /^[0-9a-f]{64}$/;

/**
 * Resolve a project-local 16-hex address against complete Library identities.
 * A prefix is only an address: it becomes a relationship after exactly one full digest matches.
 */
export function bridgeProjectAssetToLibrary(
  projectAssetId: unknown,
  libraryContentHexes: readonly string[],
): ProjectAssetLibraryBridge {
  if (typeof projectAssetId !== "string") return { status: "unaddressable" };
  const projectMatch = PROJECT_ASSET_ID.exec(projectAssetId);
  if (!projectMatch) return { status: "unaddressable" };

  const prefix = projectMatch[1]!;
  const matches = new Set<string>();
  for (const contentHex of libraryContentHexes) {
    if (!LIBRARY_CONTENT_HEX.test(contentHex)) {
      throw new TypeError("library list contains an invalid content hash");
    }
    if (contentHex.startsWith(prefix)) matches.add(`sha256:${contentHex}`);
  }

  const contentDigests = [...matches].sort();
  if (contentDigests.length === 0) return { status: "missing" };
  if (contentDigests.length === 1) {
    return { status: "matched", contentDigest: contentDigests[0]! };
  }
  return { status: "ambiguous", contentDigests };
}
