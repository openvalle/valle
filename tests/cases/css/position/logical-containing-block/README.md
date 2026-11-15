# css/position/logical-containing-block

Full product: 3 writing modes × 2 directions × 2 position modes = 12.

The transform changes no pixels but establishes the containing block for both
absolute and fixed descendants. The 4px border is excluded from its 152×92
padding box. Inline start is inset 12px; block end is inset 8px. A 40×20 logical
box becomes 20×40 in vertical writing. The static intermediate must not capture it.

The TSX and ordinary browser CSS describe the same inputs. Box expectations are
calculated from the rules above, not copied from either renderer. All tuples are
retained in the scene; no test input is randomly regenerated.
