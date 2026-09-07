//! Layered graph layout using longest-path ranks, barycenter ordering, and neighbor alignment in
//! flow/cross coordinates. Detect back edges to handle cycles without changing the displayed edge
//! directions.

use std::collections::VecDeque;

/// Return one back-edge flag per edge for cycle-aware ranking.
pub fn back_edges(n: usize, edges: &[(usize, usize)]) -> Vec<bool> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ei, &(f, t)) in edges.iter().enumerate() {
        adj[f].push(ei);
        let _ = t;
    }
    let mut state = vec![0u8; n]; // Visit states: unvisited, on the DFS stack, complete.
    let mut back = vec![false; n.max(edges.len())];
    back.truncate(edges.len());
    back.resize(edges.len(), false);
    // Iterative DFS with node and next-edge indices.
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        state[root] = 1;
        while let Some(&mut (u, ref mut i)) = stack.last_mut() {
            if *i < adj[u].len() {
                let ei = adj[u][*i];
                *i += 1;
                let v = edges[ei].1;
                match state[v] {
                    0 => {
                        state[v] = 1;
                        stack.push((v, 0));
                    }
                    1 => back[ei] = true, // An edge to a node on the stack is a back edge.
                    _ => {}
                }
            } else {
                state[u] = 2;
                stack.pop();
            }
        }
    }
    back
}

/// Assign longest-path ranks while ignoring back edges; isolated nodes remain at rank zero.
pub fn ranks(n: usize, edges: &[(usize, usize)], back: &[bool]) -> Vec<usize> {
    let mut indeg = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ei, &(f, t)) in edges.iter().enumerate() {
        if back[ei] || f == t {
            continue;
        }
        adj[f].push(t);
        indeg[t] += 1;
    }
    let mut rank = vec![0usize; n];
    let mut q: VecDeque<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    while let Some(u) = q.pop_front() {
        for &v in &adj[u] {
            rank[v] = rank[v].max(rank[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                q.push_back(v);
            }
        }
    }
    rank
}

/// Order each rank with alternating barycenter sweeps, starting from declaration order.
pub fn order(n: usize, edges: &[(usize, usize)], rank: &[usize]) -> Vec<Vec<usize>> {
    let max_rank = rank.iter().copied().max().unwrap_or(0);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for v in 0..n {
        rows[rank[v]].push(v);
    }
    let mut pos = vec![0usize; n];
    let sync = |rows: &Vec<Vec<usize>>, pos: &mut Vec<usize>| {
        for row in rows {
            for (i, &v) in row.iter().enumerate() {
                pos[v] = i;
            }
        }
    };
    sync(&rows, &mut pos);
    for sweep in 0..4 {
        let down = sweep % 2 == 0;
        let range: Vec<usize> = if down {
            (0..rows.len()).collect()
        } else {
            (0..rows.len()).rev().collect()
        };
        for &r in &range {
            let mut keyed: Vec<(f64, usize, usize)> = rows[r]
                .iter()
                .map(|&v| {
                    // Use connected neighbors in the adjacent rank for the current sweep direction.
                    let mut s = 0.0;
                    let mut c = 0usize;
                    for &(f, t) in edges.iter() {
                        let other = if f == v {
                            t
                        } else if t == v {
                            f
                        } else {
                            continue;
                        };
                        let want = if down {
                            rank[v].wrapping_sub(1)
                        } else {
                            rank[v] + 1
                        };
                        if rank[other] == want {
                            s += pos[other] as f64;
                            c += 1;
                        }
                    }
                    let key = if c == 0 { pos[v] as f64 } else { s / c as f64 };
                    (key, pos[v], v)
                })
                .collect();
            keyed.sort_by(|a, b| a.partial_cmp(b).unwrap());
            rows[r] = keyed.into_iter().map(|(_, _, v)| v).collect();
            sync(&rows, &mut pos);
        }
    }
    rows
}

/// Place ranks along the flow axis and align nodes along the cross axis. Sizes and returned centers
/// use cross/flow order.
pub fn coords(
    rows: &[Vec<usize>],
    sizes: &[(f64, f64)],
    edges: &[(usize, usize)],
    node_gap: f64,
    rank_gap: f64,
) -> Vec<(f64, f64)> {
    let n = sizes.len();
    let mut center = vec![(0.0f64, 0.0f64); n];
    // Each rank's flow extent is its largest node size.
    let mut flow_cursor = 0.0;
    for row in rows {
        let depth = row.iter().map(|&v| sizes[v].1).fold(0.0f64, f64::max);
        for &v in row {
            center[v].1 = flow_cursor + depth / 2.0;
        }
        flow_cursor += depth + rank_gap;
    }
    // Initialize cross-axis positions sequentially.
    for row in rows {
        let mut x = 0.0;
        for &v in row {
            center[v].0 = x + sizes[v].0 / 2.0;
            x += sizes[v].0 + node_gap;
        }
    }
    // Preserve order and minimum spacing, then shift each row to its desired mean position.
    for _ in 0..3 {
        for row in rows {
            let want: Vec<f64> = row
                .iter()
                .map(|&v| {
                    let mut s = 0.0;
                    let mut c = 0usize;
                    for &(f, t) in edges {
                        let other = if f == v {
                            t
                        } else if t == v {
                            f
                        } else {
                            continue;
                        };
                        s += center[other].0;
                        c += 1;
                    }
                    if c == 0 { center[v].0 } else { s / c as f64 }
                })
                .collect();
            let mut placed: Vec<f64> = Vec::with_capacity(row.len());
            let mut prev_edge = f64::NEG_INFINITY;
            for (i, &v) in row.iter().enumerate() {
                let half = sizes[v].0 / 2.0;
                let p = want[i].max(prev_edge + half);
                placed.push(p);
                prev_edge = p + half + node_gap;
            }
            let mean_w = want.iter().sum::<f64>() / want.len().max(1) as f64;
            let mean_p = placed.iter().sum::<f64>() / placed.len().max(1) as f64;
            let d = mean_p - mean_w;
            for (&v, p) in row.iter().zip(&placed) {
                center[v].0 = p - d;
            }
        }
    }
    // Straighten chain segments after mean alignment while preserving same-rank spacing.
    let mut in_nb: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut out_nb: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(f, t) in edges {
        out_nb[f].push(t);
        in_nb[t].push(f);
    }
    let mut row_of = vec![(0usize, 0usize); n];
    for (ri, row) in rows.iter().enumerate() {
        for (i, &v) in row.iter().enumerate() {
            row_of[v] = (ri, i);
        }
    }
    let try_snap = |v: usize, x: f64, center: &mut Vec<(f64, f64)>| {
        let (ri, i) = row_of[v];
        let row = &rows[ri];
        let half = sizes[v].0 / 2.0;
        if i > 0 {
            let l = row[i - 1];
            if x - half < center[l].0 + sizes[l].0 / 2.0 + node_gap {
                return;
            }
        }
        if i + 1 < row.len() {
            let r = row[i + 1];
            if x + half > center[r].0 - sizes[r].0 / 2.0 - node_gap {
                return;
            }
        }
        center[v].0 = x;
    };
    // Align downward to a single predecessor or already-aligned predecessor set. Avoid a reverse
    // pass that can pull aligned chains toward unresolved joins.
    for _ in 0..2 {
        for row in rows {
            for &v in row {
                let ps = &in_nb[v];
                let target = match ps.len() {
                    0 => continue,
                    1 => center[ps[0]].0,
                    _ => {
                        let x0 = center[ps[0]].0;
                        if ps.iter().all(|&u| (center[u].0 - x0).abs() < 1e-6) {
                            x0
                        } else {
                            continue;
                        }
                    }
                };
                try_snap(v, target, &mut center);
            }
        }
        // Align source nodes with their single successor.
        for row in rows {
            for &v in row {
                if in_nb[v].is_empty()
                    && let [u] = out_nb[v][..]
                {
                    try_snap(v, center[u].0, &mut center);
                }
            }
        }
    }
    center
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_monotonic_and_cycles_break() {
        // Cycle with a shortcut edge.
        let edges = [(0, 1), (1, 2), (2, 0), (0, 2)];
        let back = back_edges(3, &edges);
        assert_eq!(
            back.iter().filter(|&&b| b).count(),
            1,
            "remove exactly one back edge"
        );
        let r = ranks(3, &edges, &back);
        for (ei, &(f, t)) in edges.iter().enumerate() {
            if !back[ei] {
                assert!(
                    r[t] > r[f],
                    "forward edges must increase rank: {f}->{t} {r:?}"
                );
            }
        }
    }

    #[test]
    fn coords_keep_min_gap_and_no_overlap() {
        // One source connects to three nodes in the next rank.
        let edges = [(0, 1), (0, 2), (0, 3)];
        let back = back_edges(4, &edges);
        let r = ranks(4, &edges, &back);
        let rows = order(4, &edges, &r);
        let sizes = [(100.0, 40.0), (80.0, 40.0), (120.0, 40.0), (80.0, 40.0)];
        let c = coords(&rows, &sizes, &edges, 40.0, 64.0);
        let row1 = &rows[1];
        for w in row1.windows(2) {
            let (a, b) = (w[0], w[1]);
            let gap = (c[b].0 - sizes[b].0 / 2.0) - (c[a].0 + sizes[a].0 / 2.0);
            assert!(gap >= 39.9, "minimum spacing within a rank: {gap}");
        }
        // The source should align near the mean of its children.
        let mid = (c[1].0 + c[2].0 + c[3].0) / 3.0;
        assert!((c[0].0 - mid).abs() < 60.0, "{} vs {mid}", c[0].0);
    }
}
