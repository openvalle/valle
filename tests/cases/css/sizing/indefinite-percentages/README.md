# indefinite-percentages

12 combinations cross Block/Flex/Grid, definite/indefinite parent height and child minimum height. Percentage width clamps to 90px; cyclic percentage height resolves through intrinsic content.

3 layouts × 2 parent heights × 2 minima. Content width is 148px, so half is clamped to 90px. Definite content height is 88px. With auto parent height, the child contributes its 40px content or its larger minimum.

The listed inputs are permanent. Browser HTML uses ordinary CSS and does not
consume Valle layout results. Review the three browser images and native
difference images before accepting the baseline.

Grid resolves the item percentage against its final grid area after intrinsic
track sizing: a 40px area gives 20px, clamped to 24px. The track stays 40px.
Block/Flex retain the auto percentage in this indefinite-height configuration.

The matrix exposed a measured absolute auto height being treated as definite
on the final layout pass. The adapter retains its content-dependent status;
Flex recomputes that height without a percentage basis. Opposing insets,
explicit heights and aspect ratios keep their definite sizing behavior.
