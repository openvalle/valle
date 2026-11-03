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
  arc: () => {
    const a = arc();
    const sector = { innerRadius: 0, outerRadius: 100, startAngle: 0, endAngle: Math.PI / 2 };
    const ring = { innerRadius: 40, outerRadius: 100, startAngle: 1, endAngle: 4 };
    return [
      a(sector),
      a(ring),
      a({ innerRadius: 0, outerRadius: 50, startAngle: 0, endAngle: 2 * Math.PI }),
      a({ innerRadius: 30, outerRadius: 50, startAngle: 0, endAngle: 7 }),
      a({ innerRadius: 0, outerRadius: 0, startAngle: 0, endAngle: 1 }),
      a({ innerRadius: 20, outerRadius: 50, startAngle: 2, endAngle: 0.5 }),
      a.centroid(ring),
      arc().cornerRadius(8)(ring),
      arc().cornerRadius(30)(ring),
      arc().cornerRadius(10)(sector),
      arc().padAngle(0.05)(ring),
      arc().padAngle(0.05).padRadius(200)(ring),
      arc().padAngle(0.3).cornerRadius(6)({ ...ring, endAngle: 1.4 }),
      arc().innerRadius(10).outerRadius(60).startAngle(0).endAngle(5).cornerRadius(4)(),
      arc().digits(1)(ring),
      arc().cornerRadius(12)({ innerRadius: 0, outerRadius: 60, startAngle: 0, endAngle: 4.5 }),
    ];
  },
  pie: () => {
    const values = [3, 1, 4, 1, 5, -2, 0];
    return [
      pie()(values),
      pie().sort(null).sortValues(null)(values),
      pie()
        .padAngle(0.02)
        .startAngle(-Math.PI / 2)
        .endAngle(Math.PI / 2)(values),
      pie()
        .value((d) => d.n)
        .sort((a, b) => (a.name < b.name ? -1 : 1))([
        { name: "b", n: 2 },
        { name: "a", n: 7 },
      ]),
      pie().endAngle(-Math.PI)([1, 1]),
      pie()([]),
      pie()([0, 0]),
    ];
  },
  stack: () => {
    const rows = [
      { month: 1, apples: 10, pears: 5, plums: 2 },
      { month: 2, apples: 7, pears: NaN, plums: 4 },
      { month: 3, apples: 12, pears: 8, plums: 0 },
    ];
    const layers = (s) =>
      s.map((series) => ({
        key: series.key,
        index: series.index,
        points: series.map((p) => [p[0], p[1], p.data.month]),
      }));
    return [
      layers(stack().keys(["apples", "pears", "plums"])(rows)),
      layers(stack().keys(["apples", "pears", "plums"]).order([2, 0, 1])(rows)),
      layers(
        stack()
          .keys(["a", "b"])
          .value((d, key) => d[key] * 2)([
          { a: 1, b: 2 },
          { a: 3, b: 4 },
        ]),
      ),
      layers(
        stack()
          .keys((data) => Object.keys(data[0]))
          .offset(null)([{ x: 1, y: 2 }]),
      ),
      layers(stack()(rows)),
    ];
  },
  geo: () => {
    // Clockwise, as d3 takes a polygon's outside; reversed, it is the rest of the sphere.
    const square = (x, y, d) => [
      [x, y],
      [x, y + d],
      [x + d, y + d],
      [x + d, y],
      [x, y],
    ];
    const land = {
      type: "FeatureCollection",
      features: [
        { type: "Feature", geometry: { type: "Polygon", coordinates: [square(-10, 40, 20)] } },
        {
          type: "Feature",
          geometry: {
            type: "MultiPolygon",
            coordinates: [[square(100, -20, 30), square(105, -15, 10).reverse()]],
          },
        },
        // Across the antimeridian.
        { type: "Feature", geometry: { type: "Polygon", coordinates: [square(170, 50, 20)] } },
      ],
    };
    const river = {
      type: "LineString",
      coordinates: [
        [-120, 30],
        [-60, 45],
        [10, 20],
        [175, 0],
        [-170, -10],
      ],
    };
    const towns = {
      type: "MultiPoint",
      coordinates: [
        [2.35, 48.85],
        [139.7, 35.7],
        [-74, 40.7],
      ],
    };
    const usa = { type: "Polygon", coordinates: [square(-120, 30, 45)] };
    const mercator = geoMercator();
    const fitted = geoMercator().fitSize([400, 300], land);
    const framed = geoMercator().fitExtent(
      [
        [20, 10],
        [380, 290],
      ],
      river,
    );
    const turned = geoMercator().rotate([-150, 10, 20]).scale(80).translate([200, 150]);
    const clipped = geoMercator().clipExtent([
      [100, 100],
      [500, 400],
    ]);
    const albers = geoAlbers();
    const albersFit = geoAlbers().fitSize([600, 400], usa);
    const path = geoPath(fitted);
    return [
      [mercator([0, 0]), mercator([-74, 40.7]), mercator.invert([100, 50]), mercator.scale()],
      [mercator.translate(), mercator.clipExtent(), mercator.center()],
      [fitted.scale(), fitted.translate(), framed.scale(), framed.translate()],
      [turned([10, 20]), turned.invert([120, 80]), turned.rotate()],
      [albers([-96, 38]), albers([-122.4, 37.8]), albers.invert([480, 250]), albers.parallels()],
      [albers.scale(), albers.translate(), albers.rotate(), albers.center()],
      [albersFit.scale(), albersFit.translate(), albersFit([-100, 50])],
      geoMercator().angle(30).reflectY(true)([20, 30]),
      geoMercator().precision(0).precision(),
      path(land),
      path(river),
      path(towns),
      path.pointRadius(2)({ type: "Point", coordinates: [10, 10] }),
      path.pointRadius((d) => d.r)({ type: "Point", coordinates: [0, 0] }, 1),
      path.digits(1)(land.features[1]),
      path.bounds(land),
      path.area(land),
      path.centroid(land.features[0]),
      path.centroid(river),
      path.centroid(towns),
      path(null),
      geoPath(turned)(land),
      geoPath(turned)(river),
      geoPath(turned)({ type: "Polygon", coordinates: [square(-30, -25, 55).reverse()] }),
      geoPath(clipped)(land),
      geoPath(clipped)(river),
      geoPath(mercator)({ type: "Sphere" }),
      geoPath(clipped)({ type: "Sphere" }),
      geoPath(albersFit)(usa),
      geoPath(albers)({
        type: "GeometryCollection",
        geometries: [usa, towns, { type: "MultiLineString", coordinates: [river.coordinates] }],
      }),
      geoPath()({
        type: "LineString",
        coordinates: [
          [1, 2],
          [3.14159, 4],
        ],
      }),
      geoPath(geoMercator().fitWidth(300, land))(land),
      geoPath(geoMercator().fitHeight(200, river))(river),
    ];
  },
};
