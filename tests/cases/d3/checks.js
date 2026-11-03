// Calls of d3's functions whose results the built-in charts must give: the case d3/checks runs
// them on both (see crates/valle-engine/tests/cases/d3.rs).

export const checks = {
  extent: () => [
    extent([3, 1, null, 7, NaN, 2]),
    extent([]),
    extent(["b", "a", "c"]),
    extent([{ v: 4 }, { v: -2 }], (d) => d.v),
  ],
  ticks: () => [
    ticks(0, 1, 10),
    ticks(0, 100, 5),
    ticks(-3.7, 12.1, 7),
    ticks(1, 0, 4),
    ticks(0.001, 0.0093, 6),
    ticks(5, 5, 3),
    ticks(0, 1e6, 3),
    ticks(0, 10, 0),
  ],
  tickIncrement: () => [tickIncrement(0, 1, 10), tickIncrement(0, 1000, 7)],
  tickStep: () => [tickStep(0, 1, 10), tickStep(10, 0, 3)],
  nice: () => [nice(0.13, 9.87, 10), nice(-41, 997, 5), nice(5, 5, 3), nice(12, 1, 4)],
};
