# Fixed-seed layout trees

These 20 fixtures retain 1,000 trees, with 9,742 boxes and 116,904 scalar
assertions. `seeds.json` records the xorshift32 seeds and factor counts.
The generator is `tests/tools/boxes.ts`; ordinary tests use the checked-in inputs
and expectations and never generate or accept screenshots.

The bounded grammar combines three levels of block, row/column Flex and 2×2
Grid, independent direction/alignment choices, gaps, borders, padding, both box
models and fixed/percentage/calc dimensions. Expected border/padding/content
rectangles come from a separate arithmetic layout model. They are not measured
from Valle or copied from browser output. All 1,000 trees agree exactly with
Chrome, Firefox and WebKit in both geometry and pixels on the recorded capture.

This grammar deliberately has no text, fractional flex growth, absolute
children, effects or intrinsic sizing. Their hand-authored matrices cover those
behaviors. A count of 1,000 trees is not a claim of global pairwise coverage or
that every possible property combination has been tested.

```sh
bun tests/tools/boxes.ts --check  # verify inputs and expectations without writing
bun tests/tools/boxes.ts         # regenerate inputs; never overwrite browser PNGs
bun tests/tools/browser.ts css/generated  # independent candidates for review
cargo xtask cases css/generated
```

For a failure, use the printed `s<seed>-n<node>` key to locate its tree. Copy that
one tree and its ordinary browser HTML into a focused case. Remove siblings,
then declarations, rerunning both outputs until the smallest failing relation
remains. Inspect the actual and difference images, check the cited rule, and
keep the minimized regression beside the relevant feature before fixing it.
Do not accept new pictures merely because they match the current engine.
