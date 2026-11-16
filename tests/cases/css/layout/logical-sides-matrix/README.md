# css/layout/logical-sides-matrix

Full product: 3 writing modes × 2 directions = 6 cells.

Each cell is positioned, 150×150 with a 8px border and 4px padding, so its
padding box, 134×134, is the containing block of the absolutely positioned
item inside it. The item writes every flow-relative side with a different value, so a side
that receives another side's declaration shows up as a wrong box:

|         | block-start | block-end | inline-start | inline-end |
| ------- | ----------: | --------: | -----------: | ---------: |
| margin  |           3 |         5 |            9 |         11 |
| border  |           1 |         3 |            5 |          7 |
| padding |           2 |         4 |            6 |          8 |

Its inline-start inset is 7px and its block-start inset 5px; the other two are
auto. Its border box is 60px along the inline axis and 30px along the block axis,
so 60×30 in horizontal writing and 30×60 in vertical writing.

The physical side each flow-relative side becomes follows CSS Writing Modes 4: the block
axis runs top to bottom in horizontal-tb, right to left in vertical-rl and left to right
in vertical-lr; the inline axis runs left to right in horizontal-tb and top to bottom in
the vertical modes, and rtl reverses it. The item's border box starts the inset plus the
margin from the containing block's edge on each side that has an inset, and its other
edges follow from its size. Its padding and content boxes move in by the border and padding
of each physical side.

The expectations in `case.json` were calculated this way, not taken from Valle or from a
browser. The four border colors differ as well, so the pictures show which side each border
is on.
