# Fixed test resources

`NotoSans.ttf`, `NotoSansItalic.ttf` and `NotoSansSC.ttf` are generated, ignored files used by both
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
`styled.timeline.json` covers inherited and overridden run styles, decoration/stroke
resets, mixed sizes, emoji, and stable word wrapping during karaoke and typewriter
playback. The native and Web players compare these frames live.
Caption and run styles also share `textUnderlineOffset` (a CSS length or `auto`)
and `textDecorationSkipInk` (`auto`, `all`, `none`). The CSS `decoration-cjk` case
checks all five underline styles against browser captures with transparent glyph
fill, so font antialiasing cannot hide a broken decoration.

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

`shaders/` holds native WGSL inputs shared by CLI GPU assertions and Web/native
comparisons. The scene covers inline and external shaders, child/backdrop/image
reads, sparse bindings, animated uniforms, matrix layout and straight alpha.
The transition scene samples all 13 built-ins, and the Timeline tests temperature
adjustment. These have no captured PNG baselines: CLI tests assert expected
pixels and the browser compares with the native frames at runtime.

`video/` contains authored 96×64, 4 fps, one-second media fixtures (no audio).
`quadrants.mp4` has red/green on the top row and blue/yellow on the bottom row.
The `rotate90`, `rotate180`,
`rotate270` and `mirror90` files contain the same compressed frames with display
matrices; the angles follow FFmpeg's counter-clockwise convention. Native tests
assert display dimensions and corner colors, and `rotation.motion.tsx` compares
browser playback with native output, including backward seeking.

The fixtures were generated with FFmpeg 9:

```sh
ffmpeg -f lavfi -i 'color=c=red:s=96x64:r=4:d=1' \
  -vf 'drawbox=x=48:y=0:w=48:h=32:color=lime:t=fill,drawbox=x=0:y=32:w=48:h=32:color=blue:t=fill,drawbox=x=48:y=32:w=48:h=32:color=yellow:t=fill' \
  -c:v libx264 -crf 10 quadrants.mp4
ffmpeg -display_rotation 90 -i quadrants.mp4 -c copy rotate90.mp4
ffmpeg -display_rotation 180 -i quadrants.mp4 -c copy rotate180.mp4
ffmpeg -display_rotation 270 -i quadrants.mp4 -c copy rotate270.mp4
ffmpeg -display_rotation 90 -display_hflip -i quadrants.mp4 -c copy mirror90.mp4
ffmpeg -i quadrants.mp4 -pix_fmt yuv420p10le -c:v libx265 \
  -x265-params 'log-level=error:pools=1:colorprim=9:transfer=16:colormatrix=9' pq.mp4
ffmpeg -i quadrants.mp4 -pix_fmt yuv420p10le -c:v libx265 \
  -x265-params 'log-level=error:pools=1:colorprim=9:transfer=18:colormatrix=9' hlg.mp4
```

`pq.mp4` and `hlg.mp4` are 10-bit HEVC copies tagged BT.2020 with PQ and HLG.
They test detection and check warnings only, not HDR color accuracy or tone mapping.
