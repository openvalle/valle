//! Pending-node flattening and runtime context path classification.

use super::*;

pub(super) fn emit_node(
    node: PendingNode,
    nodes: &mut Vec<SceneNode>,
    children: &mut Vec<NodeId>,
    caption_rows: &mut Vec<valle_motion::caption::CaptionRows>,
    spans: &mut Vec<Span>,
    expansion_stacks: &mut Vec<Vec<String>>,
) -> NodeId {
    let id = NodeId(nodes.len() as u32);
    spans.push(node.span);
    expansion_stacks.push(node.expansion_stack);
    nodes.push(SceneNode {
        key: node.key,
        kind: node.kind,
        space: node.space,
        class_names: node.class_names,
        class_conditions: node.class_conditions,
        styles: node.styles,
        visibility: node.visibility,
        children: ChildRange::EMPTY,
        semantic: node.semantic,
    });
    let rows: Vec<Option<u32>> = node
        .children
        .iter()
        .map(|child| child.caption_row)
        .collect();
    let direct: Vec<NodeId> = node
        .children
        .into_iter()
        .map(|child| {
            emit_node(
                child,
                nodes,
                children,
                caption_rows,
                spans,
                expansion_stacks,
            )
        })
        .collect();
    let start = children.len() as u32;
    children.extend(direct);
    nodes[id.0 as usize].children = ChildRange {
        start,
        end: children.len() as u32,
    };
    // The children one `runs.map(...)` drew, side by side, are the row that repeats per run.
    let mut at = 0;
    while at < rows.len() {
        let Some(row) = rows[at] else {
            at += 1;
            continue;
        };
        let end = (at..rows.len())
            .find(|&next| rows[next] != Some(row))
            .unwrap_or(rows.len());
        caption_rows.push(valle_motion::caption::CaptionRows {
            parent: id,
            children: ChildRange {
                start: start + at as u32,
                end: start + end as u32,
            },
        });
        at = end;
    }
    id
}
