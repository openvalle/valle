# mixed-joins

16 mixed-style corner combinations cross the top edge style, unequal widths and asymmetric radii. Neighboring styles must not leave geometric holes.

4 top styles × 2 width lists × 2 radius lists. Border styles and colors are independent of radius geometry; dashed placement itself is implementation-dependent.

The listed inputs are permanent. Browser HTML uses ordinary CSS and does not
consume Valle layout results. Review the three browser images and native
difference images before accepting the baseline.

Reviewed all 16 cells and difference maps. Differences follow edge-mark phase, corner seams and coverage, without missing border sides or leaking interiors.
The strict Valle baseline prevents the browser rasterization allowance from
accepting removal of a border or shadow.
