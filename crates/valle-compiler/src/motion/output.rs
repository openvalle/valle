//! Pending-node flattening and runtime context path classification.

use super::*;

pub(super) fn emit_node(
    node: PendingNode,
    nodes: &mut Vec<SceneNode>,
    children: &mut Vec<NodeId>,
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
        styles: node.styles,
        visibility: node.visibility,
        children: ChildRange::EMPTY,
        semantic: node.semantic,
    });
    let direct: Vec<NodeId> = node
        .children
        .into_iter()
        .map(|child| emit_node(child, nodes, children, spans, expansion_stacks))
        .collect();
    let start = children.len() as u32;
    children.extend(direct);
    nodes[id.0 as usize].children = ChildRange {
        start,
        end: children.len() as u32,
    };
    id
}

pub(super) fn context_input(segments: &[String]) -> Option<ContextInput> {
    use valle_motion::PhaseKind;
    let phase = |name: &str| match name {
        "enter" => Some(PhaseKind::Enter),
        "hold" => Some(PhaseKind::Hold),
        "exit" => Some(PhaseKind::Exit),
        _ => None,
    };
    match segments {
        [name] if name == "localFrame" => Some(ContextInput::LocalFrame),
        [name] if name == "progress" => Some(ContextInput::LocalProgress),
        [name] if name == "seconds" => Some(ContextInput::CompositionSeconds),
        [name] if name == "durationFrames" => Some(ContextInput::DurationFrames),
        [fps, name] if fps == "fps" && name == "num" => Some(ContextInput::FpsNum),
        [fps, name] if fps == "fps" && name == "den" => Some(ContextInput::FpsDen),
        // Recognize per-unit fields; artifact admission restricts their use to Text perUnit
        // expressions.
        [unit, field] if unit == "unit" => match field.as_str() {
            "index" => Some(ContextInput::UnitIndex),
            "count" => Some(ContextInput::UnitCount),
            "start" => Some(ContextInput::UnitStart),
            "end" => Some(ContextInput::UnitEnd),
            _ => None,
        },
        // Viewport dimensions for numeric geometry expressions.
        [viewport, field] if viewport == "viewport" => match field.as_str() {
            "width" => Some(ContextInput::ViewportWidth),
            "height" => Some(ContextInput::ViewportHeight),
            _ => None,
        },
        [phase_name, field] => match (phase(phase_name), field.as_str()) {
            (Some(phase), "frame") => Some(ContextInput::PhaseFrame { phase }),
            (Some(phase), "durationFrames") => Some(ContextInput::PhaseDurationFrames { phase }),
            (Some(phase), "progress") => Some(ContextInput::PhaseProgress { phase }),
            (Some(phase), "elapsedFrames") => Some(ContextInput::PhaseElapsedFrames { phase }),
            (Some(phase), "active") => Some(ContextInput::PhaseActive { phase }),
            (Some(PhaseKind::Hold), "iteration") => Some(ContextInput::HoldIteration),
            (Some(PhaseKind::Hold), "cycleFrame") => Some(ContextInput::HoldCycleFrame),
            (Some(PhaseKind::Hold), "cycleProgress") => Some(ContextInput::HoldCycleProgress),
            _ => None,
        },
        _ => None,
    }
}
