//! JSON computation bridge from op/args to result/error. JavaScript wrappers only marshal data;
//! Rust owns the algorithms. A narrow string-to-string boundary keeps prepare-time behavior shared
//! and deterministic.

use serde::Deserialize;
use serde_json::{Value, json};

use super::{geo, geo_path, graph, label, noise, scale, stack};

#[derive(Deserialize)]
struct Request {
    op: String,
    #[serde(default)]
    args: Value,
}

/// Supported operator names, also listed in unknown-operator diagnostics.
pub const OPS: &[&str] = &[
    "ticks",
    "niceDomain",
    "extent",
    "extentOrDefault",
    "linear.map",
    "linear.invert",
    "band.band",
    "band.step",
    "band.bandwidth",
    "point.at",
    "point.step",
    "stack",
    "geo.project",
    "geo.path",
    "graph.layout",
    "label.place",
    "random",
    "noise1d",
    "noise2d",
];

/// Single dispatch entry point; invalid inputs return error objects rather than panicking.
pub fn dispatch(request: &str) -> String {
    let op = request_op(request);
    let outcome = run(request).and_then(|value| {
        // Null is valid only for an empty extent result.
        if op == "extent" && value.is_null() {
            return Ok(value);
        }
        // Validate output finiteness at the shared boundary because JSON serialization turns
        // non-finite floats into null.
        find_non_finite(&value).map_or(Ok(value), |path| {
            Err(format!(
                "`{op}` produced a value that is not a finite number{path}; \
                 the inputs are individually valid but the result overflowed"
            ))
        })
    });
    match outcome {
        Ok(value) => json!({ "ok": value }).to_string(),
        Err(message) => json!({ "error": message }).to_string(),
    }
}

/// Extract the operator name for diagnostics, falling back to compute.
fn request_op(request: &str) -> String {
    serde_json::from_str::<Value>(request)
        .ok()
        .and_then(|value| value.get("op").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "compute".to_owned())
}

/// Locate null values representing non-finite results and report their nested paths. The caller
/// handles the valid empty-extent exception first.
fn find_non_finite(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some(String::new()),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(at, item)| find_non_finite(item).map(|path| format!(" at [{at}]{path}"))),
        Value::Object(fields) => fields.iter().find_map(|(name, item)| {
            find_non_finite(item).map(|path| format!(" at .{name}{path}"))
        }),
        _ => None,
    }
}

fn run(request: &str) -> Result<Value, String> {
    let request: Request =
        serde_json::from_str(request).map_err(|e| format!("bad compute request: {e}"))?;
    let args = &request.args;
    match request.op.as_str() {
        "ticks" => {
            let (start, stop) = domain(args)?;
            Ok(json!(scale::ticks(start, stop, count(args)?)))
        }
        "niceDomain" => {
            let (start, stop) = domain(args)?;
            let (a, b) = scale::nice(start, stop, count(args)?);
            Ok(json!([a, b]))
        }
        "extent" => {
            let values = numbers(args, "values")?;
            Ok(match scale::extent(&values) {
                Some((min, max)) => json!([min, max]),
                None => Value::Null,
            })
        }
        "extentOrDefault" => {
            let (min, max) = domain(args)?;
            let (a, b) = scale::extent_or_default(min, max);
            Ok(json!([a, b]))
        }
        "linear.map" => Ok(json!(linear(args)?.map(number(args, "value")?))),
        "linear.invert" => Ok(json!(linear(args)?.invert(number(args, "value")?))),
        "band.band" => {
            let (start, end) = band(args)?.band(index(args)?);
            Ok(json!([start, end]))
        }
        "band.step" => Ok(json!(band(args)?.step())),
        "band.bandwidth" => Ok(json!(band(args)?.bandwidth())),
        "point.at" => Ok(json!(point(args)?.at(index(args)?))),
        "point.step" => Ok(json!(point(args)?.step())),
        "stack" => stack_values(args),
        "geo.project" => geo_project(args),
        "geo.path" => geo_path(args),
        "graph.layout" => graph_layout(args),
        "label.place" => place_labels(args),
        "random" => {
            let seed = seed(args)?;
            let n = count(args)?;
            let mut rng = noise::SeededRandom::new(seed);
            let (low, high) = args
                .get("range")
                .map(|_| pair(args, "range"))
                .transpose()?
                .unwrap_or((0.0, 1.0));
            Ok(json!(
                (0..n)
                    .map(|_| rng.next_range(low, high))
                    .collect::<Vec<_>>()
            ))
        }
        "noise1d" => Ok(json!(noise::value_noise_1d(
            seed(args)?,
            number(args, "x")?
        ))),
        "noise2d" => Ok(json!(noise::value_noise_2d(
            seed(args)?,
            number(args, "x")?,
            number(args, "y")?
        ))),
        other => Err(format!(
            "unknown compute op `{other}`; known ops: {}",
            OPS.join(", ")
        )),
    }
}

// Read arguments with diagnostics naming the invalid parameter.

fn number(args: &Value, name: &str) -> Result<f64, String> {
    args.get(name)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("`{name}` must be a finite number"))
}

fn count(args: &Value) -> Result<usize, String> {
    let value = args
        .get("count")
        .and_then(Value::as_f64)
        .ok_or_else(|| "`count` must be a number".to_string())?;
    if !value.is_finite() || !(1.0..=1000.0).contains(&value) {
        return Err("`count` must be between 1 and 1000".into());
    }
    whole("count", value)?;
    Ok(value as usize)
}

/// Validate index against count to reject off-canvas results from out-of-range indices.
fn index(args: &Value) -> Result<usize, String> {
    let value = args
        .get("index")
        .and_then(Value::as_f64)
        .ok_or_else(|| "`index` must be a number".to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err("`index` must be a non-negative number".into());
    }
    whole("index", value)?;
    let index = value as usize;
    let count = count(args)?;
    if index >= count {
        return Err(format!(
            "`index` is {index} but there are only {count} items (valid indices are 0..{})",
            count - 1
        ));
    }
    Ok(index)
}

/// Require integer-valued count parameters instead of silently truncating fractions.
fn whole(name: &str, value: f64) -> Result<(), String> {
    if value.fract() != 0.0 {
        return Err(format!("`{name}` must be a whole number, got {value}"));
    }
    Ok(())
}

fn pair(args: &Value, name: &str) -> Result<(f64, f64), String> {
    let Some(Value::Array(items)) = args.get(name) else {
        return Err(format!("`{name}` must be a two-number array"));
    };
    let [a, b] = items.as_slice() else {
        return Err(format!("`{name}` must have exactly two numbers"));
    };
    let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) else {
        return Err(format!("`{name}` must contain numbers"));
    };
    if !a.is_finite() || !b.is_finite() {
        return Err(format!("`{name}` must be finite"));
    }
    Ok((a, b))
}

fn domain(args: &Value) -> Result<(f64, f64), String> {
    // Accept either start/stop arguments or a two-element domain.
    if args.get("domain").is_some() {
        return pair(args, "domain");
    }
    Ok((number(args, "start")?, number(args, "stop")?))
}

fn numbers(args: &Value, name: &str) -> Result<Vec<f64>, String> {
    let Some(Value::Array(items)) = args.get(name) else {
        return Err(format!("`{name}` must be an array of numbers"));
    };
    items
        .iter()
        .map(|item| {
            item.as_f64()
                .ok_or_else(|| format!("`{name}` must contain only numbers"))
        })
        .collect()
}

fn linear(args: &Value) -> Result<scale::LinearScale, String> {
    Ok(scale::LinearScale::new(
        pair(args, "domain")?,
        pair(args, "range")?,
    ))
}

fn band(args: &Value) -> Result<scale::BandScale, String> {
    let range = pair(args, "range")?;
    let count = count(args)?;
    let mut band = scale::BandScale::new(range, count);
    if let Some(value) = args.get("paddingInner").and_then(Value::as_f64) {
        if !(0.0..1.0).contains(&value) {
            return Err("`paddingInner` must be in [0, 1)".into());
        }
        band.padding_inner = value;
    }
    if let Some(value) = args.get("paddingOuter").and_then(Value::as_f64) {
        if !(0.0..=1.0).contains(&value) {
            return Err("`paddingOuter` must be in [0, 1]".into());
        }
        band.padding_outer = value;
    }
    if let Some(value) = args.get("align").and_then(Value::as_f64) {
        if !(0.0..=1.0).contains(&value) {
            return Err("`align` must be in [0, 1]".into());
        }
        band.align = value;
    }
    Ok(band)
}

/// Require an explicit seed; never substitute an implicit randomness source.
fn seed(args: &Value) -> Result<u64, String> {
    let value = args
        .get("seed")
        .and_then(Value::as_f64)
        .ok_or_else(|| "`seed` must be a number — randomness has to be written down".to_string())?;
    // Reject values at or above 2^64 because u64::MAX rounds to 2^64 as f64.
    if !value.is_finite() || value < 0.0 || value >= 18_446_744_073_709_551_616.0 {
        return Err("`seed` must be a non-negative integer".into());
    }
    whole("seed", value)?;
    Ok(value as u64)
}

/// Project and fit all geographic coordinates together so they share projection parameters and
/// canvas scaling.
fn geo_project(args: &Value) -> Result<Value, String> {
    let Some(Value::Array(items)) = args.get("points") else {
        return Err("`points` must be an array of [lon, lat] pairs".into());
    };
    let mut lon_lat = Vec::with_capacity(items.len());
    for item in items {
        let Value::Array(pair) = item else {
            return Err("each point must be a [lon, lat] pair".into());
        };
        let [lon, lat] = pair.as_slice() else {
            return Err("each point must have exactly two numbers".into());
        };
        let (Some(lon), Some(lat)) = (lon.as_f64(), lat.as_f64()) else {
            return Err("point coordinates must be numbers".into());
        };
        if !lon.is_finite() || !lat.is_finite() || !(-180.0..=180.0).contains(&lon) {
            return Err("longitude must be finite and within [-180, 180]".into());
        }
        if !(-90.0..=90.0).contains(&lat) {
            return Err("latitude must be within [-90, 90]".into());
        }
        lon_lat.push((lon, lat));
    }
    if lon_lat.is_empty() {
        return Err("`points` cannot be empty — a projection needs a extent to fit".into());
    }
    let kind = match args
        .get("projection")
        .and_then(Value::as_str)
        .unwrap_or("albers")
    {
        "albers" => geo::ProjectionKind::Albers,
        "mercator" => geo::ProjectionKind::Mercator,
        other => {
            return Err(format!(
                "unknown projection `{other}`; use albers or mercator"
            ));
        }
    };
    let lon_extent = extent_of(lon_lat.iter().map(|p| p.0));
    let lat_extent = extent_of(lon_lat.iter().map(|p| p.1));
    let projector = geo::Projector::fit_kind(kind, lon_extent, lat_extent);
    let plane: Vec<(f64, f64)> = lon_lat
        .iter()
        .map(|&(lon, lat)| projector.project(lon, lat))
        .collect();
    let x = extent_of(plane.iter().map(|p| p.0));
    let y = extent_of(plane.iter().map(|p| p.1));
    let width = number(args, "width")?;
    let height = number(args, "height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err("`width` and `height` must be positive".into());
    }
    let pad = args.get("padding").and_then(Value::as_f64).unwrap_or(0.04);
    if !(0.0..0.5).contains(&pad) {
        return Err("`padding` must be in [0, 0.5)".into());
    }
    let fit = geo::Fit::contain((x.0, y.0), (x.1, y.1), width, height, pad);
    Ok(json!(
        plane
            .into_iter()
            .map(|p| {
                let (x, y) = fit.apply(p);
                json!([x, y])
            })
            .collect::<Vec<_>>()
    ))
}

/// Polygon rings → fitted SVG path data, centroids and bounds. All polygons share one projector
/// and one fit; projecting each region independently would make an incoherent map.
fn geo_path(args: &Value) -> Result<Value, String> {
    let Some(Value::Array(raw_polygons)) = args.get("polygons") else {
        return Err(
            "`polygons` must be an array of polygons, each containing rings of [lon, lat] pairs"
                .into(),
        );
    };
    if raw_polygons.is_empty() || raw_polygons.len() > 5_000 {
        return Err("geoPath needs 1..=5000 polygons".into());
    }
    let mut polygons = Vec::with_capacity(raw_polygons.len());
    let mut ring_count = 0usize;
    let mut point_count = 0usize;
    for raw_polygon in raw_polygons {
        let Value::Array(raw_rings) = raw_polygon else {
            return Err("every geoPath polygon must be an array of rings".into());
        };
        if raw_rings.is_empty() {
            return Err("every geoPath polygon needs an outer ring".into());
        }
        ring_count += raw_rings.len();
        if ring_count > 20_000 {
            return Err("geoPath is limited to 20000 rings".into());
        }
        let mut polygon = Vec::with_capacity(raw_rings.len());
        for raw_ring in raw_rings {
            let Value::Array(raw_points) = raw_ring else {
                return Err("every geoPath ring must be an array of [lon, lat] pairs".into());
            };
            if !(3..=50_000).contains(&raw_points.len()) {
                return Err("every geoPath ring needs 3..=50000 points".into());
            }
            point_count += raw_points.len();
            if point_count > 200_000 {
                return Err("geoPath is limited to 200000 total input points".into());
            }
            let mut ring = Vec::with_capacity(raw_points.len());
            for raw_point in raw_points {
                let Value::Array(pair) = raw_point else {
                    return Err("every geoPath point must be a [lon, lat] pair".into());
                };
                let [lon, lat] = pair.as_slice() else {
                    return Err("every geoPath point must have exactly two numbers".into());
                };
                let (Some(lon), Some(lat)) = (lon.as_f64(), lat.as_f64()) else {
                    return Err("geoPath coordinates must be numbers".into());
                };
                if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
                    return Err("geoPath longitude must be finite and within [-180, 180]".into());
                }
                if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
                    return Err("geoPath latitude must be finite and within [-90, 90]".into());
                }
                ring.push((lon, lat));
            }
            polygon.push(ring);
        }
        polygons.push(polygon);
    }

    let kind = match args
        .get("projection")
        .and_then(Value::as_str)
        .unwrap_or("albers")
    {
        "albers" => geo::ProjectionKind::Albers,
        "mercator" => geo::ProjectionKind::Mercator,
        other => {
            return Err(format!(
                "unknown projection `{other}`; use albers or mercator"
            ));
        }
    };
    let width = number(args, "width")?;
    let height = number(args, "height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err("`width` and `height` must be positive".into());
    }
    let padding = args.get("padding").and_then(Value::as_f64).unwrap_or(0.04);
    if !padding.is_finite() || !(0.0..0.5).contains(&padding) {
        return Err("`padding` must be in [0, 0.5)".into());
    }
    let clip = match args.get("clip") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => {
            let [left, top, right, bottom] = items.as_slice() else {
                return Err("`clip` must be [left, top, right, bottom]".into());
            };
            let values = [left, top, right, bottom]
                .map(|value| value.as_f64().filter(|value| value.is_finite()))
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| "`clip` values must be finite numbers".to_string())?;
            if values[2] <= values[0] || values[3] <= values[1] {
                return Err("`clip` right/bottom must be greater than left/top".into());
            }
            Some(geo_path::ClipRect {
                left: values[0],
                top: values[1],
                right: values[2],
                bottom: values[3],
            })
        }
        _ => return Err("`clip` must be [left, top, right, bottom]".into()),
    };
    geo_path::project_paths(&polygons, kind, width, height, padding, clip).map(|paths| json!(paths))
}

/// Prepared label anchors and measured boxes -> deterministic non-overlapping placements.
fn place_labels(args: &Value) -> Result<Value, String> {
    let Some(Value::Array(raw_candidates)) = args.get("candidates") else {
        return Err("`candidates` must be an array of { point, size, priority? } objects".into());
    };
    if raw_candidates.len() > 10_000 {
        return Err("placeLabels is limited to 10000 candidates".into());
    }

    let mut candidates = Vec::with_capacity(raw_candidates.len());
    for raw in raw_candidates {
        let Value::Object(fields) = raw else {
            return Err("every label candidate must be an object".into());
        };
        let parse_pair = |name: &str| -> Result<[f64; 2], String> {
            let Some(Value::Array(items)) = fields.get(name) else {
                return Err(format!("every label candidate needs `{name}: [x, y]`"));
            };
            let [a, b] = items.as_slice() else {
                return Err(format!(
                    "label candidate `{name}` must have exactly two numbers"
                ));
            };
            let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) else {
                return Err(format!("label candidate `{name}` values must be numbers"));
            };
            if !a.is_finite() || !b.is_finite() {
                return Err(format!("label candidate `{name}` values must be finite"));
            }
            Ok([a, b])
        };
        let point = parse_pair("point")?;
        let size = parse_pair("size")?;
        if size[0] < 0.0 || size[1] < 0.0 {
            return Err("label candidate sizes must be non-negative".into());
        }
        let priority = fields
            .get("priority")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if !priority.is_finite() {
            return Err("label candidate priority must be finite".into());
        }
        candidates.push(label::Candidate {
            point,
            size,
            priority,
        });
    }

    let Some(Value::Array(raw_bounds)) = args.get("bounds") else {
        return Err("`bounds` must be [left, top, right, bottom]".into());
    };
    let [left, top, right, bottom] = raw_bounds.as_slice() else {
        return Err("`bounds` must be [left, top, right, bottom]".into());
    };
    let bounds = [left, top, right, bottom]
        .map(|value| value.as_f64().filter(|value| value.is_finite()))
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| "`bounds` values must be finite numbers".to_string())?;
    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return Err("`bounds` right/bottom must be greater than left/top".into());
    }
    let padding = args.get("padding").and_then(Value::as_f64).unwrap_or(4.0);
    if !padding.is_finite() || padding < 0.0 {
        return Err("`padding` must be finite and non-negative".into());
    }

    Ok(json!(label::place(
        &candidates,
        [bounds[0], bounds[1], bounds[2], bounds[3]],
        padding,
    )))
}

fn extent_of(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), v| {
        (min.min(v), max.max(v))
    })
}

/// Compute graph ranks, crossing reduction, alignment, and node centers in one operation.
fn graph_layout(args: &Value) -> Result<Value, String> {
    let Some(Value::Array(sizes)) = args.get("sizes") else {
        return Err("`sizes` must be an array of [cross, flow] pairs".into());
    };
    let mut node_sizes = Vec::with_capacity(sizes.len());
    for size in sizes {
        let Value::Array(pair) = size else {
            return Err("each size must be a [cross, flow] pair".into());
        };
        let [cross, flow] = pair.as_slice() else {
            return Err("each size must have exactly two numbers".into());
        };
        let (Some(cross), Some(flow)) = (cross.as_f64(), flow.as_f64()) else {
            return Err("sizes must be numbers".into());
        };
        if !cross.is_finite()
            || !flow.is_finite()
            || !(0.0..=1_000_000.0).contains(&cross)
            || !(0.0..=1_000_000.0).contains(&flow)
        {
            return Err("sizes must be finite and within [0, 1000000]".into());
        }
        node_sizes.push((cross, flow));
    }
    let n = node_sizes.len();
    if n == 0 {
        return Err("`sizes` cannot be empty".into());
    }
    if n > 2_000 {
        return Err("graphLayout is limited to 2000 nodes".into());
    }
    let Some(Value::Array(raw_edges)) = args.get("edges") else {
        return Err("`edges` must be an array of [from, to] pairs".into());
    };
    if raw_edges.len() > 20_000 {
        return Err("graphLayout is limited to 20000 edges".into());
    }
    let mut edges = Vec::with_capacity(raw_edges.len());
    for edge in raw_edges {
        let Value::Array(pair) = edge else {
            return Err("each edge must be a [from, to] pair".into());
        };
        let [from, to] = pair.as_slice() else {
            return Err("each edge must have exactly two node indices".into());
        };
        let (Some(from), Some(to)) = (from.as_f64(), to.as_f64()) else {
            return Err("edge endpoints must be node indices".into());
        };
        // Reject invalid edge indices before they can reach the layout algorithm.
        if from.fract() != 0.0 || to.fract() != 0.0 {
            return Err("edge endpoints must be whole-number node indices".into());
        }
        if !(0.0..n as f64).contains(&from) || !(0.0..n as f64).contains(&to) {
            return Err(format!(
                "edge [{from}, {to}] references a node outside 0..{n}"
            ));
        }
        edges.push((from as usize, to as usize));
    }
    let node_gap = args.get("nodeGap").and_then(Value::as_f64).unwrap_or(40.0);
    let rank_gap = args.get("rankGap").and_then(Value::as_f64).unwrap_or(64.0);
    if !node_gap.is_finite()
        || !rank_gap.is_finite()
        || !(0.0..=1_000_000.0).contains(&node_gap)
        || !(0.0..=1_000_000.0).contains(&rank_gap)
    {
        return Err("`nodeGap` and `rankGap` must be finite and within [0, 1000000]".into());
    }
    let back = graph::back_edges(n, &edges);
    let ranks = graph::ranks(n, &edges, &back);
    let rows = graph::order(n, &edges, &ranks);
    let centers = graph::coords(&rows, &node_sizes, &edges, node_gap, rank_gap);
    Ok(json!({
        "ranks": ranks,
        "rows": rows,
        "backEdges": back,
        "centers": centers.into_iter().map(|(cross, flow)| json!([cross, flow])).collect::<Vec<_>>(),
    }))
}

fn point(args: &Value) -> Result<scale::PointScale, String> {
    Ok(scale::PointScale::new(pair(args, "range")?, count(args)?))
}

fn stack_values(args: &Value) -> Result<Value, String> {
    let Some(Value::Array(raw_series)) = args.get("values") else {
        return Err("`values` must be a series-major array of number arrays".into());
    };
    if raw_series.is_empty() || raw_series.len() > 100 {
        return Err("stack needs 1..=100 series".into());
    }
    let mut values = Vec::with_capacity(raw_series.len());
    let mut categories = None;
    for raw in raw_series {
        let Value::Array(items) = raw else {
            return Err("every stack series must be an array of numbers".into());
        };
        let expected = *categories.get_or_insert(items.len());
        if items.len() != expected {
            return Err(
                "stack values must be rectangular; every series needs the same category count"
                    .into(),
            );
        }
        if items.is_empty() || items.len() > 10_000 {
            return Err("each stack series needs 1..=10000 categories".into());
        }
        values.push(
            items
                .iter()
                .map(|item| {
                    item.as_f64()
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| "stack values must contain only finite numbers".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    if values.len() * categories.unwrap_or(0) > 100_000 {
        return Err("stack is limited to 100000 total cells".into());
    }
    let offset = match args.get("offset").and_then(Value::as_str).unwrap_or("zero") {
        "zero" => stack::StackOffset::Zero,
        "expand" => stack::StackOffset::Expand,
        other => {
            return Err(format!(
                "unknown stack offset `{other}`; use zero or expand"
            ));
        }
    };
    stack::stack(&values, offset).map(|result| json!(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(op: &str, args: Value) -> Value {
        let raw = dispatch(&json!({ "op": op, "args": args }).to_string());
        let value: Value = serde_json::from_str(&raw).expect("valid json");
        assert!(value.get("error").is_none(), "unexpected error: {value}");
        value["ok"].clone()
    }

    fn err(op: &str, args: Value) -> String {
        let raw = dispatch(&json!({ "op": op, "args": args }).to_string());
        let value: Value = serde_json::from_str(&raw).expect("valid json");
        value["error"].as_str().expect("an error").to_string()
    }

    // Invalid-input and output-overflow checks.

    /// Reject overflowing results even when every input is finite.
    #[test]
    fn a_result_that_overflows_is_an_error_not_a_silent_null() {
        let message = err(
            "linear.map",
            json!({ "domain": [0, 1e-308], "range": [0, 1e308], "value": 1e10 }),
        );
        assert!(
            message.contains("not a finite number") && message.contains("linear.map"),
            "the author must be told which op overflowed: {message}"
        );

        // Report the path of nested invalid results.
        let message = err(
            "random",
            json!({ "seed": 1, "count": 3, "range": [-1e308, 1e308] }),
        );
        assert!(
            message.contains("not a finite number") && message.contains("[0]"),
            "the path to the bad number must be in the message: {message}"
        );
    }

    /// An empty extent's null result remains valid.
    #[test]
    fn an_empty_extent_is_still_allowed_to_be_null() {
        assert_eq!(ok("extent", json!({ "values": [] })), Value::Null);
    }

    /// Indices must stay within the declared count.
    #[test]
    fn an_index_past_the_end_is_rejected_instead_of_extrapolated() {
        let message = err(
            "band.band",
            json!({ "range": [0, 300], "count": 3, "index": 7 }),
        );
        assert!(
            message.contains("only 3 items"),
            "say how many there actually are: {message}"
        );
        let message = err(
            "point.at",
            json!({ "range": [0, 300], "count": 4, "index": 9 }),
        );
        assert!(message.contains("only 4 items"), "{message}");
    }

    /// Reject the rounded floating-point representation of the u64 seed ceiling.
    #[test]
    fn a_seed_at_two_to_the_sixty_fourth_is_rejected_not_saturated() {
        let message = err(
            "random",
            json!({ "seed": 18_446_744_073_709_551_616.0_f64, "count": 1 }),
        );
        assert!(message.contains("non-negative integer"), "{message}");
    }

    /// Reject fractional count and seed parameters rather than truncating them.
    #[test]
    fn fractional_counts_and_seeds_are_rejected_rather_than_truncated() {
        for (op, args) in [
            ("random", json!({ "seed": 3.7, "count": 2 })),
            ("noise1d", json!({ "seed": 3.7, "x": 0.5 })),
            (
                "band.band",
                json!({ "range": [0, 300], "count": 3.9, "index": 0 }),
            ),
            (
                "band.band",
                json!({ "range": [0, 300], "count": 3, "index": 1.5 }),
            ),
        ] {
            let message = err(op, args);
            assert!(
                message.contains("whole number"),
                "`{op}` must reject a fractional count/seed/index: {message}"
            );
        }
    }

    #[test]
    fn the_bridge_returns_the_same_numbers_as_the_rust_api() {
        // Bridge results must match direct Rust computation.
        assert_eq!(
            ok("ticks", json!({ "start": 0, "stop": 100, "count": 5 })),
            json!(scale::ticks(0.0, 100.0, 5))
        );
        assert_eq!(
            ok(
                "niceDomain",
                json!({ "start": 0.1, "stop": 0.9, "count": 5 })
            ),
            json!([0.0, 1.0])
        );
        assert_eq!(
            ok(
                "linear.map",
                json!({ "domain": [0, 10], "range": [300, 100], "value": 5 })
            ),
            json!(200.0)
        );
        assert_eq!(
            ok(
                "band.band",
                json!({ "range": [0, 300], "count": 3, "index": 2 })
            ),
            json!([200.0, 300.0])
        );
    }

    #[test]
    fn map_helpers_cross_the_json_bridge_and_bad_shapes_fail_closed() {
        let paths = ok(
            "geo.path",
            json!({
                "polygons": [[[[170, 10], [-170, 10], [-170, -10], [170, -10]]]],
                "projection": "mercator",
                "width": 640,
                "height": 360,
                "padding": 0.05
            }),
        );
        assert_eq!(paths.as_array().map(Vec::len), Some(1));
        assert!(
            paths[0]["d"]
                .as_str()
                .is_some_and(|path| path.ends_with('Z'))
        );

        let labels = ok(
            "label.place",
            json!({
                "candidates": [
                    { "point": [100, 100], "size": [40, 16], "priority": 2 },
                    { "point": [100, 100], "size": [40, 16], "priority": 1 }
                ],
                "bounds": [0, 0, 200, 200],
                "padding": 4
            }),
        );
        assert_eq!(labels.as_array().map(Vec::len), Some(2));
        assert_eq!(labels[0]["visible"], true);
        assert_eq!(labels[1]["visible"], true);
        assert_ne!(labels[0], labels[1]);

        assert!(err("geo.path", json!({ "polygons": [] })).contains("1..=5000"));
        assert!(
            err(
                "label.place",
                json!({ "candidates": [{ "point": [0, 0], "size": [-1, 2] }], "bounds": [0, 0, 10, 10] })
            )
            .contains("non-negative")
        );
    }

    #[test]
    fn bad_input_becomes_an_error_never_a_panic_and_never_a_guess() {
        // Diagnostics must identify the invalid argument.
        assert!(err("ticks", json!({ "start": 0, "stop": 100 })).contains("`count`"));
        assert!(err("ticks", json!({ "start": "x", "stop": 1, "count": 5 })).contains("`start`"));
        assert!(
            err(
                "linear.map",
                json!({ "domain": [0], "range": [0, 1], "value": 0 })
            )
            .contains("exactly two")
        );
        assert!(
            err(
                "band.band",
                json!({ "range": [0, 1], "count": 2, "index": 0, "paddingInner": 1.5 })
            )
            .contains("paddingInner")
        );
        assert!(
            err(
                "graph.layout",
                json!({ "sizes": [[100, 40], [100, 40]], "edges": [[0.5, 1]] })
            )
            .contains("whole-number")
        );
        assert!(
            err(
                "graph.layout",
                json!({ "sizes": [[1000001, 40]], "edges": [] })
            )
            .contains("1000000")
        );
        // Unknown operators report the supported names.
        let unknown = err("scaleLog", json!({}));
        assert!(
            unknown.contains("scaleLog") && unknown.contains("ticks"),
            "{unknown}"
        );
    }

    #[test]
    fn graph_layout_enforces_public_node_edge_and_gap_budgets() {
        let too_many_nodes = vec![json!([10, 10]); 2_001];
        assert!(
            err(
                "graph.layout",
                json!({ "sizes": too_many_nodes, "edges": [] })
            )
            .contains("2000 nodes")
        );

        let too_many_edges = vec![json!([0, 0]); 20_001];
        assert!(
            err(
                "graph.layout",
                json!({ "sizes": [[10, 10]], "edges": too_many_edges })
            )
            .contains("20000 edges")
        );

        for args in [
            json!({ "sizes": [[10, 10]], "edges": [], "nodeGap": 1000001 }),
            json!({ "sizes": [[10, 10]], "edges": [], "rankGap": -1 }),
        ] {
            assert!(
                err("graph.layout", args).contains("1000000"),
                "graph gaps must use the documented finite range"
            );
        }
    }

    #[test]
    fn a_non_finite_domain_is_rejected_rather_than_producing_nan_geometry() {
        // Reject invalid geographic domains before producing non-finite geometry.
        assert!(
            err(
                "linear.map",
                json!({ "domain": [0, null], "range": [0, 1], "value": 0 })
            )
            .contains("`domain`")
        );
    }
}
