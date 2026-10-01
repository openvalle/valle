//! Extract a template's transitive expression closure, including lexical captures.

use super::*;

pub(super) fn extract(arena: &ExprArena, template: &mut PendingNode) -> Option<ExprArena> {
    let mut roots = Vec::new();
    if !visit(template, &mut |id| roots.push(*id)) {
        return None;
    }
    let mut live = vec![false; arena.values.len()];
    while let Some(id) = roots.pop() {
        let at = id.0 as usize;
        if !*live.get(at)? {
            live[at] = true;
            roots.extend(arena.values[at].children());
        }
    }
    let mut remap = vec![None; arena.values.len()];
    let mut compact = ExprArena::default();
    for (at, expr) in arena.values.iter().enumerate().filter(|(at, _)| live[*at]) {
        remap[at] = Some(ExprId(compact.values.len() as u32));
        let mut expr = expr.clone();
        // The arena is topological, so every dependency has already been copied.
        expr.remap_children(|id| remap[id.0 as usize].expect("captured dependency"));
        compact.values.push(expr);
        compact.types.push(arena.types[at]);
        compact.spans.push(arena.spans[at]);
        compact
            .expansion_stacks
            .push(arena.expansion_stacks[at].clone());
        compact.reads_runtime.push(arena.reads_runtime[at]);
    }
    visit(template, &mut |id| {
        *id = remap[id.0 as usize].expect("captured template root");
    });
    Some(compact)
}

// Use the same visitor to collect and rewrite roots. Only kinds admitted by the
// instance renderer are supported; other kinds must retain ordinary lowering.
fn visit(node: &mut PendingNode, f: &mut impl FnMut(&mut ExprId)) -> bool {
    for expr in node.class_conditions.values_mut() {
        f(expr);
    }
    for style in &mut node.styles {
        if let StyleValue::Expr { expr } = &mut style.value {
            f(expr);
        }
    }
    if let Some(expr) = &mut node.visibility {
        f(expr);
    }
    match &mut node.kind {
        NodeKind::Box | NodeKind::Group => {}
        NodeKind::Text {
            text,
            per_unit: None,
            path: None,
        } => {
            if let TextValue::Expr { expr } = text {
                f(expr);
            }
        }
        NodeKind::Path {
            d,
            fill,
            stroke,
            trim_start,
            trim_end,
            ..
        } => {
            if let PathValue::Expr { expr, .. } = d {
                f(expr);
            }
            if let Some(fill) = fill {
                paint(fill, f);
            }
            if let Some(stroke) = stroke {
                paint(&mut stroke.paint, f);
                number(&mut stroke.width, f);
                number(&mut stroke.dash_offset, f);
            }
            number(trim_start, f);
            number(trim_end, f);
        }
        _ => return false,
    }
    node.children.iter_mut().all(|child| visit(child, f))
}

fn number(value: &mut NumberValue, f: &mut impl FnMut(&mut ExprId)) {
    if let NumberValue::Expr { expr } = value {
        f(expr);
    }
}

fn point(value: &mut PointValue, f: &mut impl FnMut(&mut ExprId)) {
    if let PointValue::Expr { expr } = value {
        f(expr);
    }
}

fn color(value: &mut ColorValue, f: &mut impl FnMut(&mut ExprId)) {
    if let ColorValue::Expr { expr } = value {
        f(expr);
    }
}

fn paint(value: &mut PaintValue, f: &mut impl FnMut(&mut ExprId)) {
    let stops = match value {
        PaintValue::Solid { color: value } => {
            color(value, f);
            return;
        }
        PaintValue::Linear {
            start, end, stops, ..
        } => {
            point(start, f);
            point(end, f);
            stops
        }
        PaintValue::Radial {
            center,
            radius,
            stops,
            ..
        } => {
            point(center, f);
            number(radius, f);
            stops
        }
        PaintValue::Conic {
            center,
            start_angle,
            stops,
            ..
        } => {
            point(center, f);
            number(start_angle, f);
            stops
        }
    };
    for stop in stops {
        number(&mut stop.offset, f);
        color(&mut stop.color, f);
    }
}
