# visibility-backdrop

Six controls distinguish visible, zero opacity, inherited hidden, display:none and explicitly visible children of a hidden filtered box, with normal and multiply blending. A hidden box must not paint its backdrop; visible descendants still paint.

This is the full product of the factors in `CASES`, not random sampling.
Parent rectangles have independent numerical expectations. The three browser
captures and native/difference pictures are reviewed before accepting pixels.

## Browser decision

The inherited-hidden and display:none controls paint only the page. The visible
child still paints. For the final hidden-parent/multiply case the engines differ:
Chrome blends the child with an inverted backdrop, Firefox with the page, and
WebKit produces a third result. We select Firefox and record this as an interop
choice rather than claiming consensus. CSS2 visibility says the hidden box is
invisible while descendants may restore visibility. The broader interaction is
tracked by [FXTF issue 455](https://github.com/w3c/fxtf-drafts/issues/455),
which explicitly describes these three different behaviors and proposes treating
the backdrop as a background effect. The issue remains open; this is not a settled
new normative rule.

The engine previously painted the hidden box's backdrop even without a visible
child. `over_backdrop` now skips that box's backdrop while preserving its visible
descendants and their blend group. Native and Firefox differ by one output level
over 640 pixels; the other captures remain available for future review.
