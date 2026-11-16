# intrinsic-constraints

12 combinations cross intrinsic size keywords, maximum size and row/column layout. Controls expose clamping and wrapping independently on both physical axes.

3 intrinsic keywords × 2 limits × 2 axes. The unconstrained main-axis sum is 44 + 8 + 68 = 120px; maximum constraints change the available line size.

The listed inputs are permanent. Browser HTML uses ordinary CSS and does not
consume Valle layout results. Review the three browser images and native
difference images before accepting the baseline.

The matrix exposed max-width replacing a min/max-content measurement constraint
in Taffy flex line collection. The layout adapter now measures the intrinsic
keyword before the parent clamps it. The 150px min-content control stays 68px;
the 90px max-content control clamps the 120px intrinsic width to 90px.
