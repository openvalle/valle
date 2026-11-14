//! Packs repeated absolute leaf boxes before their variant enters style and layout.

use crate::template::{Instances, Kind, Node, Op, Piece, Prop, Template, V};
use std::rc::Rc;

/// Pack consecutive sibling leaves from the same JSX site. The original expression program
/// and guards remain, including expressions that can fail before a value reaches a row.
pub(crate) fn pack(t: &mut Template) {
    if !t.pictures.is_empty() || t.program.ops.iter().any(|op| matches!(op, Op::Bounds(..))) {
        return;
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
        return;
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
    let eligible = |n: &Node| {
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
            if !eligible(first) {
                i += 1;
                continue;
            }
            let mut end = i + 1;
            while end < children.len() {
                let n = &t.nodes[children[end] as usize];
                if !eligible(n)
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
        t.instances
            .push((first as u32, Rc::new(Instances { fields, rows })));
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
        return;
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
}
