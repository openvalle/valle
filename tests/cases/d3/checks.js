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
  scaleLinear: () => {
    const x = scaleLinear().domain([0, 100]).range([0, 640]);
    const y = scaleLinear([0, 10], [300, 0]);
    const piecewise = scaleLinear().domain([-1, 0, 1]).range([0, 50, 200]);
    const clamped = scaleLinear().domain([10, 20]).range([0, 1]).clamp(true);
    const rounded = scaleLinear().domain([0, 3]).rangeRound([0, 100]);
    const nice = scaleLinear().domain([0.13, 97.6]).nice();
    const descending = scaleLinear().domain([5, -5]).range([0, 10]);
    return [
      [x(0), x(37.5), x(100), x(-20), x(120), x.invert(320), x(null), x("12")],
      [y(2.5), y.invert(75), y.ticks(), y.ticks(4), y.domain(), y.range()],
      [piecewise(-0.5), piecewise(0.25), piecewise(2), piecewise.invert(125)],
      [clamped(5), clamped(15), clamped(25), clamped.invert(2), clamped.clamp()],
      [rounded(1), rounded(2), nice.domain(), nice.ticks(5), descending(2.5), descending.ticks(3)],
      scaleLinear().domain([0, 7]).nice(4).domain(),
      scaleLinear().domain([3, 3]).range([0, 10])(3),
      scaleLinear().unknown(-1)(NaN),
      x.copy().domain([0, 10])(5),
    ];
  },
  scaleLinearColors: () => {
    const c = scaleLinear().domain([0, 1]).range(["red", "steelblue"]);
    const alpha = scaleLinear().domain([0, 1]).range(["rgba(0, 0, 0, 0)", "#ff8000"]);
    const hsl = scaleLinear().domain([0, 1]).range(["hsl(120, 50%, 50%)", "white"]);
    const text = scaleLinear().domain([0, 1]).range(["0px 4px", "10px 8px"]);
    const named = scaleLinear().domain([0, 1]).range(["transparent", "blue"]);
    return [c(0), c(0.3), c(1), alpha(0.25), hsl(0.5), text(0.5), named(0.5), c(1.5)];
  },
  scaleBand: () => {
    const x = scaleBand().domain(["a", "b", "c", "d"]).range([0, 400]);
    const padded = scaleBand(["a", "b", "c"], [0, 300]).padding(0.2);
    const parts = scaleBand().domain(["a", "b", "c"]).range([300, 0]).paddingInner(0.1);
    const outer = scaleBand().domain(["a", "b"]).range([0, 100]).paddingOuter(0.5).align(0);
    const round = scaleBand().domain(["a", "b", "c"]).rangeRound([0, 100]).padding(0.1);
    return [
      [x("a"), x("c"), x("e"), x.bandwidth(), x.step(), x.domain()],
      [padded("b"), padded.bandwidth(), padded.padding()],
      [parts("a"), parts("c"), parts.bandwidth()],
      [outer("a"), outer("b"), outer.bandwidth(), outer.align()],
      [round("a"), round("b"), round("c"), round.bandwidth(), round.round()],
      scaleBand().domain(["a", "a", "b"]).range([0, 10]).domain(),
      x.copy().range([0, 40])("b"),
    ];
  },
  scalePoint: () => {
    const x = scalePoint().domain(["a", "b", "c"]).range([0, 100]);
    const padded = scalePoint(["a", "b", "c", "d"], [0, 90]).padding(0.5);
    return [
      [x("a"), x("b"), x("c"), x.bandwidth(), x.step()],
      [padded("a"), padded("d"), padded.padding(), padded.step()],
      x.copy().range([0, 10])("b"),
    ];
  },
  scaleSequential: () => {
    const s = scaleSequential((t) => t * t).domain([0, 10]);
    const ranged = scaleSequential([0, 100], ["white", "black"]);
    const clamped = scaleSequential((t) => t)
      .domain([10, 0])
      .clamp(true);
    return [
      [s(5), s(10), s(-5), s.domain(), s.ticks(5)],
      [ranged(25), ranged.range()],
      [clamped(2), clamped(-4), clamped(12)],
      scaleSequential((t) => t).domain([3, 3])(3),
    ];
  },
  scaleQuantize: () => {
    const q = scaleQuantize().domain([0, 1]).range(["a", "b", "c"]);
    const n = scaleQuantize([0, 100], [1, 2, 3, 4]);
    return [
      [q(0), q(0.3), q(0.34), q(0.5), q(0.9), q(1.2), q(-1)],
      [q.thresholds(), q.invertExtent("b"), q.invertExtent("a"), q.invertExtent("z")],
      [n(24), n(25), n(26), n.thresholds(), n.ticks(4)],
      scaleQuantize().domain([0.1, 9.7]).nice().domain(),
    ];
  },
  line: () => {
    const points = [
      [0, 10],
      [10.12345, 40],
      [20, 25],
      [35, 25],
      [50, 60],
    ];
    const rows = [
      { t: 0, v: 3 },
      { t: 1, v: null },
      { t: 2, v: 5 },
      { t: 3, v: 4 },
      { t: 4, v: 9 },
    ];
    const gappy = line()
      .x((d) => d.t * 20)
      .y((d) => d.v * 10)
      .defined((d) => d.v != null);
    return [
      line()(points),
      line().curve(curveMonotoneX)(points),
      line().digits(1)(points),
      line().digits(null)(points),
      gappy(rows),
      gappy.curve(curveMonotoneX)(rows),
      line()([]),
      line()([[1, 2]]),
      line().curve(curveMonotoneX)([
        [0, 0],
        [5, 5],
      ]),
      line(
        (d) => d.x,
        (d) => d.y,
      ).curve(curveMonotoneX)([
        { x: 0, y: 5 },
        { x: 1, y: 5 },
        { x: 2, y: 9 },
        { x: 3, y: 1 },
        { x: 3, y: 1 },
        { x: 6, y: 8 },
      ]),
    ];
  },
  area: () => {
    const points = [
      [0, 10],
      [10, 40],
      [20, 25],
      [30, 50],
    ];
    const band = area()
      .x((d) => d[0])
      .y0((d) => d[1] - 5)
      .y1((d) => d[1] + 5);
    return [
      area()(points),
      area().y0(100)(points),
      area().curve(curveMonotoneX).y0(60)(points),
      band(points),
      band.lineY1()(points),
      band.lineY0()(points),
      area()
        .defined((d) => d[1] !== 40)
        .y0(0)(points),
      area()
        .x0(0)
        .x1((d) => d[0])(points),
    ];
  },
};
