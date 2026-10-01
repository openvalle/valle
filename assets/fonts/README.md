# Shared fonts

This directory is the source of truth for fonts bundled with OpenValle.
Native, Web, tests, and benchmarks use these same files. Copies under build
outputs or benchmark public directories are generated from here.

| Directory | Fonts | Purpose and provenance |
| --- | --- | --- |
| [noto](noto) | 13 faces | Variable sans and CJK, static test faces, monospace, symbols, math, and color emoji. Keep each supplied OFL file with its font. [Sources and pinned revisions](noto/README.md) are recorded separately. |
| [katex](katex) | 19 faces | Mathematical typesetting. See the [font attribution](katex/KaTeX-fonts-NOTICE.txt) and [OFL](katex/OFL.txt). Exact font hashes are locked in `crates/valle-motion/src/math_formula/fonts.rs`. |

Font binaries are unmodified; their embedded metadata retains version and
copyright information. All bundled fonts are under SIL OFL 1.1.
RaTeX code has a separate [MIT license](../../crates/valle-motion/licenses/ratex/RaTeX-MIT.txt).

Registration and embedding remain in `valle-motion`. The default Motion pack lists its served `/runtime/fonts/` files in
`crates/valle-motion/src/text/mod.rs`.
