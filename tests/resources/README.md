# Fixed test resources

`NotoSans.ttf` and `NotoSansItalic.ttf` are generated, ignored files used by both
the Rust text renderer and the browser's `@font-face`. Their URLs and SHA-256
hashes come directly from `BUILT_IN` in
[`text.rs`](../../crates/valle-engine/src/text.rs). The SIL Open Font License 1.1
is retained in `OFL.txt`.

Scene tests and `bun tests/tools/browser.ts` prepare these files automatically.
To prepare them alone, run `cargo xtask fonts` from the repository root.
Valid local files are reused; missing or invalid ones are copied from
`$VALLE_CACHE_DIR/files/` (by default `~/.valle/cache/files/`). A cache miss
downloads the pinned file. Both cached and downloaded bytes are checked before
being copied here. Preparation errors fail the command; no system font is used
as a fallback. A damaged cache file must be removed before retrying.

`counter.mp4`, `photo.png`, `rings.png` and `tile.png` are fixed video/image inputs
restored from the earlier test suite. `timeline/` holds Motion components and
the nested Timeline shared by the Timeline cases. Shared references use
`/tests/resources/...`, mapped to this repository by the test loader and browser
server, so moving a case does not change its resource paths. Media bytes are
preserved.

`timeline/captions/` holds inputs shared by numerical Rust subtitle tests and live
Web/native rendering checks. These have no committed PNG baselines. `words.motion.tsx`
is a caption presenter with timed runs, configurable color and an intro/outro;
its paths resolve beside the Timeline that uses it.

`svg/pixels.png` is a shared 4×2 opaque color grid authored for object-fit and
integer `pixelated` scaling. Dedicated SVG drawings live beside their file cases
at `../cases/svg/<feature>/file/source.svg`, rather than in this shared directory.
They are authored test inputs, not copied from WPT or browser screenshots.
The embedded-image case embeds the same 4×2 PNG as a data URI. Embedded-formats
uses an authored 16×8 grid as PNG, JPEG (quality 100, no chroma subsampling) and
lossless WebP to exercise each SVG image decoder.

Once prepared or cached, fonts need no network access. HTML uses explicit
whitespace entities in the text case so formatting the source cannot collapse
the tested spaces or newline.
