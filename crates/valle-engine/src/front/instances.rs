//! Packs repeated leaf boxes and restores individual nodes where a scene observes them.

use crate::template::{Error, Instances, Kind, Op, Part, Piece, Prop, Tag, Template, V};
use std::rc::Rc;

/// Pack consecutive sibling leaves from the same JSX site. The original expression program
/// and guards remain, including expressions that can fail before a value reaches a row.
pub(crate) fn pack(t: &mut Template) -> Result<(), Error> {
    if !t.pictures.is_empty() || t.program.ops.iter().any(|op| matches!(op, Op::Bounds(..))) {
        return expand(t, |_| true);
    }
    // SVG references inspect their subtrees; ordinary box tables remain independent of them.
    if t.nodes.iter().any(|n| {
        matches!(
            n.kind,
            Kind::Svg {
                tag: Tag::Use | Tag::Defs | Tag::ClipPath | Tag::Mask | Tag::Symbol | Tag::Marker,
                ..
            }
        )
    }) {
        expand(t, |kind| matches!(kind, Kind::Svg { .. }))?;
    }
    // A 3D scene sorts individual planes. A table must not hide those planes from its sorter.
    if t.nodes.iter().any(|n| {
        n.style
            .iter()
            .any(|(k, _)| matches!(&**k, "perspective" | "transformStyle"))
            || n.classes.iter().any(|c| {
                c.contains("perspective") || c.contains("preserve-3d") || c.contains("transform-3d")
            })
    }) {
        return expand(t, |_| true);
    }
    let mut numeric = Vec::with_capacity(t.program.ops.len());
    for op in &t.program.ops {
        let yes = match op {
            Op::Const(V::Num(_))
            | Op::Seconds
            | Op::Progress
            | Op::Bin(..)
            | Op::Neg(_)
            | Op::Math(..)
            | Op::Noise(..)
            | Op::Wiggle(..)
            | Op::Spring { .. }
            | Op::Particle(..) => true,
            Op::Select(_, a, b) => numeric[*a as usize] && numeric[*b as usize],
            _ => false,
        };
        numeric.push(yes);
    }
    let eligible = |i: u32| {
        if t.instances.iter().any(|(node, _)| *node == i) {
            return false;
        }
        let n = &t.nodes[i as usize];
        if n.kind != Kind::View
            || !n.children.is_empty()
            || n.site.is_none()
            || n.classes.iter().any(|c| &**c != "absolute")
        {
            return false;
        }
        let has = |name: &str| n.style.iter().find(|(k, _)| &**k == name).map(|(_, p)| p);
        let absolute = n.classes.iter().any(|c| &**c == "absolute")
            || matches!(has("position"), Some(Prop::Const(V::Str(s))) if &**s == "absolute");
        if !absolute
            || matches!(has("position"), Some(p) if !matches!(p, Prop::Const(V::Str(s)) if &**s == "absolute"))
        {
            return false;
        }
        for name in ["width", "height"] {
            if !matches!(has(name), Some(Prop::Const(V::Num(x))) if x.is_finite() && (0.0..=33_554_431.0).contains(x))
            {
                return false;
            }
        }
        for name in ["left", "top"] {
            if !matches!(has(name), Some(Prop::Const(V::Num(_))))
                && !matches!(has(name), Some(Prop::Expr(e)) if numeric[*e as usize])
            {
                return false;
            }
        }
        n.style.iter().all(|(k, _)| {
            matches!(
                &**k,
                "position"
                    | "width"
                    | "height"
                    | "left"
                    | "top"
                    | "borderRadius"
                    | "backgroundColor"
                    | "opacity"
                    | "translate"
                    | "rotate"
                    | "scale"
                    | "transform"
                    | "transformOrigin"
            )
        })
    };
    let mut groups = Vec::new();
    for children in std::iter::once(&t.roots).chain(t.nodes.iter().map(|n| &n.children)) {
        let mut i = 0;
        while i < children.len() {
            let first = &t.nodes[children[i] as usize];
            if !eligible(children[i]) {
                i += 1;
                continue;
            }
            let mut end = i + 1;
            while end < children.len() {
                let n = &t.nodes[children[end] as usize];
                if !eligible(children[end])
                    || n.site != first.site
                    || !n
                        .style
                        .iter()
                        .map(|(k, _)| k)
                        .eq(first.style.iter().map(|(k, _)| k))
                {
                    break;
                }
                end += 1;
            }
            if end - i >= 2 {
                groups.push(children[i..end].to_vec());
            }
            i = end;
        }
    }
    let mut removed = vec![false; t.nodes.len()];
    for group in groups {
        let first = group[0] as usize;
        let fields = t.nodes[first]
            .style
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        let rows = group
            .iter()
            .map(|&i| {
                t.nodes[i as usize]
                    .style
                    .iter()
                    .map(|(_, v)| v.clone())
                    .collect()
            })
            .collect();
        t.instances.push((
            first as u32,
            Rc::new(Instances {
                fields,
                rows,
                formula: None,
            }),
        ));
        let node = &mut t.nodes[first];
        node.path = None;
        node.classes.clear();
        node.style = vec![("position".into(), Prop::Const(V::Str("absolute".into())))];
        node.style.extend(
            ["left", "top", "width", "height"].map(|k| (k.into(), Prop::Const(V::Num(0.0)))),
        );
        for &i in &group[1..] {
            removed[i as usize] = true;
        }
    }
    if t.instances.is_empty() {
        return Ok(());
    }
    let mut next = 0;
    let ids: Vec<u32> = removed
        .iter()
        .map(|&gone| {
            let i = next;
            next += u32::from(!gone);
            i
        })
        .collect();
    let remap = |list: &mut Vec<u32>| {
        list.retain(|i| !removed[*i as usize]);
        for i in list {
            *i = ids[*i as usize];
        }
    };
    remap(&mut t.roots);
    for (i, _) in &mut t.instances {
        *i = ids[*i as usize];
    }
    for n in &mut t.nodes {
        remap(&mut n.children);
        match &mut n.kind {
            Kind::Mask { source, .. } => *source = ids[*source as usize],
            Kind::Text { content } | Kind::Span { content, .. } => {
                for p in content {
                    if let Piece::Span(i) = p {
                        *i = ids[*i as usize];
                    }
                }
            }
            _ => {}
        }
    }
    let mut i = 0;
    t.nodes.retain(|_| {
        let keep = !removed[i];
        i += 1;
        keep
    });
    Ok(())
}

/// Restore independent nodes where their keys or 3D planes are observable. Expressions still
/// come from the shared formula, but retain the ordinary layout and painting path.
fn expand(t: &mut Template, select: impl Fn(&Kind) -> bool) -> Result<(), Error> {
    if t.instances.is_empty() {
        return Ok(());
    }
    let mut nodes = Vec::new();
    let mut ids = Vec::new();
    let clock = crate::eval::Clock {
        seconds: 0.0,
        progress: 0.0,
    };
    for (i, node) in t.nodes.iter().enumerate() {
        let first = nodes.len() as u32;
        if let Some((_, table)) = t
            .instances
            .iter()
            .find(|(n, b)| *n as usize == i && b.formula.is_some() && select(&node.kind))
        {
            let f = table.formula.as_ref().unwrap();
            for row in &table.rows {
                let mut local = Vec::new();
                let mut mapped = Vec::new();
                for p in row {
                    let Prop::Const(v) = p else {
                        return Err("instance input must be constant".into());
                    };
                    mapped.push(t.program.ops.len() as u32);
                    t.program.ops.push(Op::Const(v.clone()));
                    local.push(v.clone());
                }
                mapped.extend(&f.imports);
                let mut dynamic = vec![false; local.len()];
                for &e in &f.imports {
                    let fixed = match &t.program.ops[e as usize] {
                        Op::Const(v) => Some(v.clone()),
                        _ => None,
                    };
                    dynamic.push(fixed.is_none());
                    local.push(fixed.unwrap_or(V::Num(0.0)));
                }
                for op in f.program.ops.iter().skip(mapped.len()) {
                    let mut changes = false;
                    op.operands(|e| changes |= dynamic[e as usize]);
                    dynamic.push(changes);
                    let value = if changes {
                        V::Num(0.0)
                    } else {
                        crate::eval::value(op, &local, clock)
                            .map_err(|e| format!("{}: TypeError: {e}", node.place()))?
                    };
                    local.push(value.clone());
                    if !changes {
                        mapped.push(t.program.ops.len() as u32);
                        t.program.ops.push(Op::Const(value));
                        continue;
                    }
                    let at = |i: u32| mapped[i as usize];
                    let op = match op {
                        Op::Const(v) => Op::Const(v.clone()),
                        Op::Bin(b, a, c) => Op::Bin(*b, at(*a), at(*c)),
                        Op::Cmp(b, a, c) => Op::Cmp(*b, at(*a), at(*c)),
                        Op::Neg(a) => Op::Neg(at(*a)),
                        Op::Select(a, b, c) => Op::Select(at(*a), at(*b), at(*c)),
                        Op::Math(f, args) => Op::Math(*f, args.iter().map(|e| at(*e)).collect()),
                        Op::Noise(seed, args) => {
                            Op::Noise(*seed, args.iter().map(|e| at(*e)).collect())
                        }
                        Op::Interp(interp) => {
                            let mut interp = (**interp).clone();
                            interp.input = at(interp.input);
                            for (input, output) in &mut interp.stops {
                                *input = at(*input);
                                *output = at(*output);
                            }
                            Op::Interp(Rc::new(interp))
                        }
                        Op::Template(parts) => Op::Template(
                            parts
                                .iter()
                                .map(|p| match p {
                                    Part::Text(s) => Part::Text(s.clone()),
                                    Part::Expr(e) => Part::Expr(at(*e)),
                                })
                                .collect(),
                        ),
                        _ => return Err("unsupported instance expression".into()),
                    };
                    mapped.push(t.program.ops.len() as u32);
                    t.program.ops.push(op);
                }
                let mut node = node.clone();
                let fields = table
                    .fields
                    .iter()
                    .cloned()
                    .zip(f.outputs.iter().map(|p| match p {
                        Prop::Const(v) => Prop::Const(v.clone()),
                        Prop::Expr(e) => match &t.program.ops[mapped[*e as usize] as usize] {
                            Op::Const(v) => Prop::Const(v.clone()),
                            _ => Prop::Expr(mapped[*e as usize]),
                        },
                    }))
                    .collect();
                match &mut node.kind {
                    Kind::Svg { attributes, .. } => attributes.extend(fields),
                    _ => node.style = fields,
                }
                node.path = f.key.as_ref().map(|p| {
                    let v = match p {
                        Prop::Const(v) => v,
                        Prop::Expr(e) => &local[*e as usize],
                    };
                    if f.prefix.is_empty() {
                        v.text().into_owned().into()
                    } else {
                        format!("{}/{}", f.prefix, v.text()).into()
                    }
                });
                nodes.push(node);
            }
        } else {
            nodes.push(node.clone());
        }
        ids.push(first..nodes.len() as u32);
    }
    let remap = |children: &mut Vec<u32>| {
        *children = children
            .iter()
            .flat_map(|i| ids[*i as usize].clone())
            .collect();
    };
    for n in &mut nodes {
        remap(&mut n.children);
        match &mut n.kind {
            Kind::Mask { source, .. } => *source = ids[*source as usize].start,
            Kind::Text { content } | Kind::Span { content, .. } => {
                for p in content {
                    if let Piece::Span(i) = p {
                        *i = ids[*i as usize].start;
                    }
                }
            }
            _ => {}
        }
    }
    for (n, _) in &mut t.pictures {
        *n = ids[*n as usize].start;
    }
    t.instances
        .retain(|(i, b)| b.formula.is_none() || !select(&t.nodes[*i as usize].kind));
    for (n, _) in &mut t.instances {
        *n = ids[*n as usize].start;
    }
    remap(&mut t.roots);
    t.nodes = nodes;
    Ok(())
}
