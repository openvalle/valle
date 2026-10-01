//! Read-only, random-access Motion quality checks over final screen geometry.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;
use valle_timeline::FrameRate;

use super::{LayoutCache, LayoutOptions, PreparedScene, build_tree};
use crate::{NodeKind, ResolvedProps, motion_context_at_frame};

const STROBE_EXTENT_FRACTION: f64 = 0.25;
const MIN_FAST_INTERVALS: u32 = 3;
const MIN_READING_SECONDS: f64 = 0.3;
const READING_SECONDS_PER_WORD: f64 = 0.3;
const READING_SECONDS_PER_CJK_CHARACTER: f64 = 0.225;
const MIN_STROBE_EQUIVALENT_SIZE: f64 = 4.0;
const MAIN_ACTION_AREA_FRACTION: f64 = 0.01;
const MIN_MAIN_ACTION_INTERVALS: u32 = 3;
const LINEAR_AREA_FRACTION: f64 = 0.005;
const MIN_LINEAR_INTERVALS: u32 = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionReview {
    pub frames_analyzed: u32,
    pub nodes_analyzed: usize,
    pub issues: Vec<MotionReviewIssue>,
    pub trajectories: Vec<MotionTrajectory>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionTrajectory {
    pub node: String,
    pub points: Vec<MotionTrajectoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionTrajectoryPoint {
    pub frame: u32,
    /// Final screen anchor and bounds, in output pixels.
    pub position: [f64; 2],
    pub bounds: [f64; 4],
    /// Absent for the first visible frame after a gap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity_px_per_second: Option<[f64; 2]>,
    /// Absent until two consecutive velocity samples exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceleration_px_per_second2: Option<[f64; 2]>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionReviewIssue {
    pub code: &'static str,
    pub severity: &'static str,
    pub node: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_node: Option<String>,
    /// Inclusive frame interval of the detected issue.
    pub start_frame: u32,
    pub end_frame: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_displacement_px_per_frame: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_px_per_frame: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected_rows: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub example_keys: Vec<String>,
    pub message: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct FastRun {
    start_frame: u32,
    end_frame: u32,
    peak: f64,
    peak_threshold: f64,
}

#[derive(Debug, Clone)]
struct TextRun {
    recommended_seconds: f64,
    start_frame: u32,
    end_frame: u32,
    text: String,
    word_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct FrameRun {
    start_frame: u32,
    end_frame: u32,
}

#[derive(Debug, Clone, Copy)]
struct LinearRun {
    frames: FrameRun,
    delta: [f64; 2],
}

fn emit_run(node: &str, run: FastRun, issues: &mut Vec<MotionReviewIssue>) {
    if run.end_frame - run.start_frame < MIN_FAST_INTERVALS {
        return;
    }
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "strobe",
        severity: "warning",
        node: node.to_owned(),
        other_node: None,
        start_frame: run.start_frame,
        end_frame: run.end_frame,
        peak_displacement_px_per_frame: Some(run.peak),
        threshold_px_per_frame: Some(run.peak_threshold),
        visible_seconds: None,
        recommended_seconds: None,
        word_count: None,
        message: "Visible content moves over a quarter of its size along its travel direction for at least three frames without motion blur; consider motionBlur: \"auto\" or a slower move.",
    });
}

fn merge_batch_strobes(issues: &mut Vec<MotionReviewIssue>, groups: &BTreeMap<String, String>) {
    let mut grouped = BTreeMap::<String, (MotionReviewIssue, BTreeSet<String>)>::new();
    let mut output = Vec::new();
    for issue in issues.drain(..) {
        let Some(group) = groups.get(&issue.node).filter(|_| issue.code == "strobe") else {
            output.push(issue);
            continue;
        };
        if let Some((summary, rows)) = grouped.get_mut(group) {
            rows.insert(issue.node.clone());
            summary.start_frame = summary.start_frame.min(issue.start_frame);
            summary.end_frame = summary.end_frame.max(issue.end_frame);
            if issue.peak_displacement_px_per_frame > summary.peak_displacement_px_per_frame {
                summary.peak_displacement_px_per_frame = issue.peak_displacement_px_per_frame;
                summary.threshold_px_per_frame = issue.threshold_px_per_frame;
            }
        } else {
            let rows = BTreeSet::from([issue.node.clone()]);
            grouped.insert(group.clone(), (issue, rows));
        }
    }
    for (group, (mut issue, rows)) in grouped {
        issue.node = group;
        issue.affected_rows = Some(rows.len());
        issue.example_keys = rows.into_iter().take(3).collect();
        issue.message = "Visible rows in this batch move fast relative to their size; review the example rows or add motion blur.";
        output.push(issue);
    }
    *issues = output;
}

fn reading_words(text: &str) -> u32 {
    let mut words = 0_u32;
    let mut in_word = false;
    for ch in text.chars() {
        let single_character_word = matches!(ch,
            '\u{3400}'..='\u{9fff}' | '\u{20000}'..='\u{2fa1f}'
            | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7a3}'
        );
        if single_character_word {
            words = words.saturating_add(1);
            in_word = false;
        } else if ch.is_alphanumeric() {
            if !in_word {
                words = words.saturating_add(1);
            }
            in_word = true;
        } else {
            in_word = false;
        }
    }
    words
}

fn reading_seconds(text: &str) -> f64 {
    let cjk = text
        .chars()
        .filter(|ch| {
            matches!(ch,
                '\u{3400}'..='\u{9fff}' | '\u{20000}'..='\u{2fa1f}'
                | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7a3}'
            )
        })
        .count() as u32;
    MIN_READING_SECONDS
        + READING_SECONDS_PER_WORD * f64::from(reading_words(text).saturating_sub(cjk))
        + READING_SECONDS_PER_CJK_CHARACTER * f64::from(cjk)
}

fn emit_text_run(node: &str, run: TextRun, fps: FrameRate, issues: &mut Vec<MotionReviewIssue>) {
    let visible_seconds =
        f64::from(run.end_frame - run.start_frame + 1) / crate::frame_rate_as_f64(fps);
    let recommended_seconds = run.recommended_seconds;
    if visible_seconds + 1.0e-9 >= recommended_seconds {
        return;
    }
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "text_readability",
        severity: "warning",
        node: node.to_owned(),
        other_node: None,
        start_frame: run.start_frame,
        end_frame: run.end_frame,
        peak_displacement_px_per_frame: None,
        threshold_px_per_frame: None,
        visible_seconds: Some(visible_seconds),
        recommended_seconds: Some(recommended_seconds),
        word_count: Some(run.word_count),
        message: "Text is visible for less than the reading estimate (0.3 seconds plus 0.3 seconds per word or 0.225 seconds per CJK character); keep it on screen longer.",
    });
}

fn emit_out_of_frame(node: &str, run: FrameRun, issues: &mut Vec<MotionReviewIssue>) {
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "text_out_of_frame",
        severity: "warning",
        node: node.to_owned(),
        other_node: None,
        start_frame: run.start_frame,
        end_frame: run.end_frame,
        peak_displacement_px_per_frame: None,
        threshold_px_per_frame: None,
        visible_seconds: None,
        recommended_seconds: None,
        word_count: None,
        message: "Visible text extends beyond the output frame; move or resize it.",
    });
}

fn emit_overlap(nodes: &(String, String), run: FrameRun, issues: &mut Vec<MotionReviewIssue>) {
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "text_overlap",
        severity: "warning",
        node: nodes.0.clone(),
        other_node: Some(nodes.1.clone()),
        start_frame: run.start_frame,
        end_frame: run.end_frame,
        peak_displacement_px_per_frame: None,
        threshold_px_per_frame: None,
        visible_seconds: None,
        recommended_seconds: None,
        word_count: None,
        message: "Visible text boxes overlap; adjust their layout or timing.",
    });
}

fn emit_motion_while_reading(
    nodes: &(String, String),
    run: FrameRun,
    issues: &mut Vec<MotionReviewIssue>,
) {
    if run.end_frame - run.start_frame + 1 < MIN_MAIN_ACTION_INTERVALS {
        return;
    }
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "motion_while_reading",
        severity: "info",
        node: nodes.0.clone(),
        other_node: Some(nodes.1.clone()),
        start_frame: run.start_frame,
        end_frame: run.end_frame,
        peak_displacement_px_per_frame: None,
        threshold_px_per_frame: None,
        visible_seconds: None,
        recommended_seconds: None,
        word_count: None,
        message: "A prominent action moves while text is being read; consider separating their timing.",
    });
}

fn emit_simultaneous_actions(
    nodes: &(String, String),
    run: FrameRun,
    issues: &mut Vec<MotionReviewIssue>,
) {
    if run.end_frame - run.start_frame + 1 < MIN_MAIN_ACTION_INTERVALS {
        return;
    }
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "simultaneous_main_actions",
        severity: "info",
        node: nodes.0.clone(),
        other_node: Some(nodes.1.clone()),
        start_frame: run.start_frame,
        end_frame: run.end_frame,
        peak_displacement_px_per_frame: None,
        threshold_px_per_frame: None,
        visible_seconds: None,
        recommended_seconds: None,
        word_count: None,
        message: "Two prominent, separate actions move at the same time; check their visual hierarchy.",
    });
}

fn emit_linear_motion(
    node: &str,
    run: LinearRun,
    viewport: [f64; 2],
    issues: &mut Vec<MotionReviewIssue>,
) {
    let intervals = run.frames.end_frame - run.frames.start_frame;
    if intervals < MIN_LINEAR_INTERVALS
        || valle_draw::math::hypot(run.delta[0], run.delta[1]) * f64::from(intervals)
            < viewport[0].min(viewport[1]) * 0.1
    {
        return;
    }
    issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
        code: "linear_motion",
        severity: "info",
        node: node.to_owned(),
        other_node: None,
        start_frame: run.frames.start_frame,
        end_frame: run.frames.end_frame,
        peak_displacement_px_per_frame: None,
        threshold_px_per_frame: None,
        visible_seconds: None,
        recommended_seconds: None,
        word_count: None,
        message: "A prominent subject travels at nearly constant speed for many frames; check whether this move needs easing.",
    });
}

fn visible_area(bounds: [f64; 4], viewport: [f64; 2]) -> f64 {
    let width = bounds[2].min(viewport[0]) - bounds[0].max(0.0);
    let height = bounds[3].min(viewport[1]) - bounds[1].max(0.0);
    width.max(0.0) * height.max(0.0)
}

fn numbered_group(key: &str) -> Option<(&str, u32)> {
    let separator = key.rfind(['-', '_'])?;
    let (prefix, suffix) = key.split_at(separator + 1);
    if prefix.len() < 2 || suffix.len() > 8 || suffix.starts_with('0') && suffix.len() > 1 {
        return None;
    }
    Some((prefix, suffix.parse().ok()?))
}

fn emit_stagger_timing(
    candidates: &BTreeSet<String>,
    onsets: &HashMap<String, u32>,
    fps: FrameRate,
    issues: &mut Vec<MotionReviewIssue>,
) {
    let mut groups = BTreeMap::<String, Vec<(u32, String, Option<u32>)>>::new();
    for key in candidates {
        if let Some((prefix, index)) = numbered_group(key) {
            groups.entry(prefix.to_owned()).or_default().push((
                index,
                key.clone(),
                onsets.get(key).copied(),
            ));
        }
    }
    let rate = crate::frame_rate_as_f64(fps);
    for mut group in groups.into_values() {
        if !(3..=64).contains(&group.len()) || group.iter().any(|entry| entry.2.is_none()) {
            continue;
        }
        group.sort_by_key(|entry| entry.0);
        if group
            .windows(2)
            .any(|pair| pair[0].0.checked_add(1) != Some(pair[1].0))
        {
            continue;
        }
        let starts = group
            .iter()
            .map(|entry| entry.2.expect("checked above"))
            .collect::<Vec<_>>();
        let ascending = starts.windows(2).all(|pair| pair[0] <= pair[1]);
        let descending = starts.windows(2).all(|pair| pair[0] >= pair[1]);
        let earliest = starts.iter().min().copied().unwrap();
        let latest = starts.iter().max().copied().unwrap();
        if (!ascending && !descending)
            || earliest == latest
            || f64::from(latest - earliest) > rate * 2.0
        {
            continue;
        }
        if !starts.windows(2).any(|pair| {
            let gap_ms = f64::from(pair[0].abs_diff(pair[1])) * 1000.0 / rate;
            !(30.0..=100.0).contains(&gap_ms)
        }) {
            continue;
        }
        issues.push(MotionReviewIssue {
        affected_rows: None,
        example_keys: Vec::new(),
            code: "stagger_timing",
            severity: "info",
            node: group[0].1.clone(),
            other_node: Some(group[group.len() - 1].1.clone()),
            start_frame: earliest,
            end_frame: latest,
            peak_displacement_px_per_frame: None,
            threshold_px_per_frame: None,
            visible_seconds: None,
            recommended_seconds: None,
            word_count: None,
            message: "A numbered group begins moving with stagger gaps outside 30–100 ms; review the rhythm.",
        });
    }
}

fn main_action_area_and_bounds(
    before: &super::scene::ReviewNodeSample,
    after: &super::scene::ReviewNodeSample,
    viewport: [f64; 2],
    jump_limit: f64,
) -> Option<(f64, [f64; 4])> {
    if !before.visible || !before.on_screen || !after.visible || !after.on_screen {
        return None;
    }
    let bounds = after.bounds;
    let width = (bounds[2] - bounds[0]).max(0.0);
    let height = (bounds[3] - bounds[1]).max(0.0);
    let area = visible_area(bounds, viewport);
    let dx = after.position.x - before.position.x;
    let dy = after.position.y - before.position.y;
    let distance = valle_draw::math::hypot(dx, dy);
    (area.is_finite()
        && area >= viewport[0] * viewport[1] * MAIN_ACTION_AREA_FRACTION
        && distance.is_finite()
        && distance <= jump_limit
        && distance > (width.min(height) * 0.1).max(4.0))
    .then_some((area, bounds))
}

fn separate_boxes(left: [f64; 4], right: [f64; 4]) -> bool {
    left[2] <= right[0] || right[2] <= left[0] || left[3] <= right[1] || right[3] <= left[1]
}

fn update_frame_runs<K: Ord + Clone>(
    active: &mut BTreeMap<K, FrameRun>,
    observed: &BTreeSet<K>,
    frame: u32,
    mut finished: impl FnMut(&K, FrameRun),
) {
    let ended = active
        .keys()
        .filter(|key| !observed.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    for key in ended {
        if let Some(run) = active.remove(&key) {
            finished(&key, run);
        }
    }
    for key in observed {
        active
            .entry(key.clone())
            .and_modify(|run| run.end_frame = frame)
            .or_insert(FrameRun {
                start_frame: frame,
                end_frame: frame,
            });
    }
}

// Only numeric tokens may vary; changing words still starts a new reading interval.
fn numeric_text_signature(text: &str) -> String {
    let mut result = String::new();
    let mut numeric = false;
    for ch in text.chars() {
        if ch.is_numeric() {
            if !numeric {
                result.push('\u{fffc}');
            }
            numeric = true;
        } else if numeric && matches!(ch, ',' | '.' | '，' | '．') {
            // Decimal and grouping separators are part of a numeric counter.
        } else {
            numeric = false;
            result.push(ch);
        }
    }
    result
}

fn painted_nodes(prepared: &PreparedScene) -> BTreeSet<String> {
    let artifact = prepared.artifact();
    let intrinsic = |node: &crate::SceneNode| {
        matches!(
            node.kind,
            NodeKind::Text { .. }
                | NodeKind::Path { .. }
                | NodeKind::Image { .. }
                | NodeKind::Video { .. }
                | NodeKind::Scene3D { .. }
                | NodeKind::ShaderLayer { .. }
                | NodeKind::MathFormula { .. }
                | NodeKind::Glass(_)
        )
    };
    let mut keys: BTreeSet<_> = artifact
        .nodes
        .iter()
        .filter(|node| intrinsic(node))
        .map(|node| node.key.clone())
        .collect();
    for group in &artifact.instance_groups {
        let mut templates = vec![&group.template];
        let mut children = group.template_children.iter().collect::<Vec<_>>();
        while let Some(child) = children.pop() {
            templates.push(&child.node);
            children.extend(&child.children);
        }
        for template in templates.into_iter().filter(|node| intrinsic(node)) {
            for row in 0..group.rows() {
                if let Some(key) = group.key_for_node(row, &template.key) {
                    keys.insert(key);
                }
            }
        }
    }
    keys
}

/// Inspect every output frame using the same post-layout anchors and resolved blur filters as
/// rendering. The caller can impose an explicit frame budget; every admitted
/// frame is inspected, without silently skipping long compositions.
pub fn review_motion(
    prepared: &PreparedScene,
    props: &ResolvedProps,
    opts: &LayoutOptions<'_>,
    fps: FrameRate,
    duration_frames: u32,
    host_duration: valle_timeline::RationalTime,
    max_frames: u32,
    include_trajectories: bool,
) -> Result<MotionReview, String> {
    if duration_frames == 0 || max_frames < 2 || duration_frames > max_frames {
        return Err(format!(
            "review needs 1..={max_frames} output frames; use --max-frames to cover this composition"
        ));
    }
    let intrinsic = painted_nodes(prepared);
    let mut candidates = intrinsic.clone();
    let mut text_nodes = prepared
        .artifact()
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, NodeKind::Text { .. }))
        .map(|node| node.key.clone())
        .collect::<BTreeSet<_>>();
    for group in &prepared.artifact().instance_groups {
        if matches!(group.template.kind, NodeKind::Text { .. }) {
            for row in 0..group.rows() {
                if let Some(key) = group.key_for_node(row, &group.template.key) {
                    text_nodes.insert(key);
                }
            }
        }
        let mut children = group.template_children.iter().collect::<Vec<_>>();
        while let Some(child) = children.pop() {
            if matches!(child.node.kind, NodeKind::Text { .. }) {
                for row in 0..group.rows() {
                    if let Some(key) = group.key_for_node(row, &child.node.key) {
                        text_nodes.insert(key);
                    }
                }
            }
            children.extend(&child.children);
        }
    }
    let jump_limit = opts
        .viewport
        .size
        .width
        .zip(opts.viewport.size.height)
        .map_or(f64::INFINITY, |(width, height)| {
            f64::from(width.max(height))
        });
    let cache = LayoutCache::new(prepared, opts.fonts);
    let faces = crate::FaceCache::new();
    let mut previous = HashMap::<String, super::scene::ReviewNodeSample>::new();
    let mut groups = BTreeMap::<String, String>::new();
    let mut active = BTreeMap::<String, FastRun>::new();
    let mut active_text = BTreeMap::<String, TextRun>::new();
    let mut active_out_of_frame = BTreeMap::<String, FrameRun>::new();
    let mut active_overlaps = BTreeMap::<(String, String), FrameRun>::new();
    let mut active_reading_actions = BTreeMap::<(String, String), FrameRun>::new();
    let mut active_simultaneous_actions = BTreeMap::<(String, String), FrameRun>::new();
    let mut active_linear = BTreeMap::<String, LinearRun>::new();
    let mut last_delta = HashMap::<String, [f64; 2]>::new();
    let mut motion_onsets = HashMap::<String, u32>::new();
    let mut trajectories = BTreeMap::<String, Vec<MotionTrajectoryPoint>>::new();
    let mut issues = Vec::new();
    let source_frames = prepared
        .artifact()
        .composition
        .as_ref()
        .map(|composition| composition.duration_frames(fps))
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or(duration_frames);
    for frame in 0..duration_frames {
        let mut ctx = motion_context_at_frame(frame.min(source_frames - 1), source_frames, fps)
            .ok_or_else(|| format!("review frame {frame} is outside the composition"))?;
        ctx.host = crate::MotionHostContext::new(
            crate::time::sample_time_at_frame(i64::from(frame), fps)
                .map_err(|error| error.to_string())?,
            host_duration,
            frame,
            duration_frames,
        )
        .ok_or_else(|| format!("review frame {frame} has an invalid host interval"))?;
        let tree = match &cache {
            Some(cache) => cache.build_tree(&ctx, props, opts.viewport, opts.styles),
            None => build_tree(prepared, &ctx, props, opts),
        }
        .map_err(|error| format!("review frame {frame}: {error}"))?;
        let mut current = tree.review_node_samples();
        let text_ink = if text_nodes.is_empty() {
            HashMap::new()
        } else {
            crate::emit::review_text_ink(&tree, &faces)
                .map_err(|error| format!("review frame {frame}: {error}"))?
        };
        for node in &text_nodes {
            // Flow text is shaped inside its parent's inline layout and may not
            // own a box at all. Its emitted source key is still authoritative.
            if let Some(ink) = text_ink.get(node) {
                current
                    .entry(node.clone())
                    .or_insert_with(|| super::scene::ReviewNodeSample {
                        opacity: f64::from(ink.opacity),
                        cycle: 0,
                        group: None,
                        painted: true,
                        position: valle_draw::Point::new(0.0, 0.0),
                        bounds: [0.0; 4],
                        visible: true,
                        on_screen: true,
                        motion_blurred: ink.motion_blurred,
                    });
            }
            if let Some(sample) = current.get_mut(node) {
                let Some(lines) = text_ink
                    .get(node)
                    .map(|ink| &ink.lines)
                    .filter(|lines| !lines.is_empty())
                else {
                    sample.visible = false;
                    continue;
                };
                let bounds = lines.iter().fold(
                    [
                        f64::INFINITY,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::NEG_INFINITY,
                    ],
                    |b, line| {
                        [
                            b[0].min(line.left()),
                            b[1].min(line.top()),
                            b[2].max(line.right()),
                            b[3].max(line.bottom()),
                        ]
                    },
                );
                sample.bounds = bounds;
                sample.position = valle_draw::Point::new(
                    (bounds[0] + bounds[2]) * 0.5,
                    (bounds[1] + bounds[3]) * 0.5,
                );
                sample.on_screen = opts
                    .viewport
                    .size
                    .width
                    .zip(opts.viewport.size.height)
                    .is_none_or(|(w, h)| {
                        bounds[2] > 0.0
                            && bounds[3] > 0.0
                            && bounds[0] < f64::from(w)
                            && bounds[1] < f64::from(h)
                    });
            }
        }
        for (key, sample) in &mut current {
            let painted = sample.painted || intrinsic.contains(key);
            sample.visible &= painted;
            if painted {
                candidates.insert(key.clone());
            }
        }
        for (key, sample) in &current {
            if let Some(group) = &sample.group {
                groups.insert(key.clone(), group.clone());
            }
        }
        // A reborn particle is a new trajectory even when its semantic key stays
        // stable. Also handle source-frame expressions that jump across cycles.
        previous.retain(|key, before| {
            current
                .get(key)
                .is_some_and(|after| before.cycle == after.cycle)
        });
        last_delta.retain(|key, _| previous.contains_key(key));
        if include_trajectories {
            let rate = crate::frame_rate_as_f64(fps);
            for node in &candidates {
                let Some(sample) = current
                    .get(node)
                    .filter(|sample| sample.visible && sample.on_screen)
                else {
                    continue;
                };
                let points = trajectories.entry(node.clone()).or_default();
                let before = points
                    .last()
                    .filter(|point| point.frame + 1 == frame && previous.contains_key(node));
                let velocity = before.map(|point| {
                    [
                        (sample.position.x - point.position[0]) * rate,
                        (sample.position.y - point.position[1]) * rate,
                    ]
                });
                let acceleration = before
                    .and_then(|point| point.velocity_px_per_second)
                    .zip(velocity)
                    .map(|(before, after)| {
                        [(after[0] - before[0]) * rate, (after[1] - before[1]) * rate]
                    });
                points.push(MotionTrajectoryPoint {
                    frame,
                    position: [sample.position.x, sample.position.y],
                    bounds: sample.bounds,
                    velocity_px_per_second: velocity,
                    acceleration_px_per_second2: acceleration,
                });
            }
        }
        let mut visible_text = Vec::new();
        for node in &text_nodes {
            let sample = current.get(node);
            let text = tree
                .node_texts
                .get(node)
                .map(String::as_str)
                .filter(|text| reading_words(text) > 0);
            if let (Some(sample), Some(_)) = (sample, text)
                && sample.visible
            {
                for line in text_ink.get(node).into_iter().flat_map(|ink| &ink.lines) {
                    visible_text.push((
                        node.clone(),
                        [line.left(), line.top(), line.right(), line.bottom()],
                        sample.on_screen,
                    ));
                }
            }
            let displayed = sample
                .filter(|sample| sample.visible && sample.on_screen)
                .and(text);
            let previous_run = active_text.remove(node);
            if let Some(text) = displayed {
                if let Some(mut run) = previous_run {
                    if run.text == text
                        || numeric_text_signature(&run.text) == numeric_text_signature(text)
                    {
                        run.end_frame = frame;
                        run.word_count = run.word_count.max(reading_words(text));
                        run.recommended_seconds =
                            run.recommended_seconds.max(reading_seconds(text));
                        active_text.insert(node.clone(), run);
                        continue;
                    }
                    emit_text_run(node, run, fps, &mut issues);
                }
                active_text.insert(
                    node.clone(),
                    TextRun {
                        recommended_seconds: reading_seconds(text),
                        start_frame: frame,
                        end_frame: frame,
                        text: text.to_owned(),
                        word_count: reading_words(text),
                    },
                );
            } else if let Some(run) = previous_run {
                emit_text_run(node, run, fps, &mut issues);
            }
        }
        let mut out_of_frame = BTreeSet::new();
        if let Some((width, height)) = opts.viewport.size.width.zip(opts.viewport.size.height) {
            for (node, bounds, _) in &visible_text {
                if bounds[0] < -1.0
                    || bounds[1] < -1.0
                    || bounds[2] > f64::from(width) + 1.0
                    || bounds[3] > f64::from(height) + 1.0
                {
                    out_of_frame.insert(node.clone());
                }
            }
        }
        update_frame_runs(
            &mut active_out_of_frame,
            &out_of_frame,
            frame,
            |node, run| emit_out_of_frame(node, run, &mut issues),
        );
        visible_text.retain(|(_, _, on_screen)| *on_screen);
        visible_text
            .sort_by(|left, right| left.1[0].total_cmp(&right.1[0]).then(left.0.cmp(&right.0)));
        let mut overlaps = BTreeSet::new();
        for (index, (left_node, left_bounds, _)) in visible_text.iter().enumerate() {
            for (right_node, right_bounds, _) in visible_text.iter().skip(index + 1) {
                if right_bounds[0] >= left_bounds[2] - 1.0 {
                    break;
                }
                if left_node == right_node {
                    continue;
                }
                let overlap_width =
                    left_bounds[2].min(right_bounds[2]) - left_bounds[0].max(right_bounds[0]);
                let overlap_height =
                    left_bounds[3].min(right_bounds[3]) - left_bounds[1].max(right_bounds[1]);
                if overlap_width > 1.0 && overlap_height > 1.0 {
                    let pair = if left_node < right_node {
                        (left_node.clone(), right_node.clone())
                    } else {
                        (right_node.clone(), left_node.clone())
                    };
                    overlaps.insert(pair);
                }
            }
        }
        update_frame_runs(&mut active_overlaps, &overlaps, frame, |nodes, run| {
            emit_overlap(nodes, run, &mut issues)
        });
        if frame > 0 {
            let viewport = opts
                .viewport
                .size
                .width
                .zip(opts.viewport.size.height)
                .map_or([f64::INFINITY; 2], |(width, height)| {
                    [f64::from(width), f64::from(height)]
                });
            let mut moving = candidates
                .iter()
                .filter(|node| !text_nodes.contains(*node))
                .filter_map(|node| {
                    previous
                        .get(node)
                        .zip(current.get(node))
                        .and_then(|(before, after)| {
                            main_action_area_and_bounds(before, after, viewport, jump_limit)
                        })
                        .map(|(area, bounds)| (node.clone(), area, bounds))
                })
                .collect::<Vec<_>>();
            moving.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            // Main-action hints deliberately inspect only the largest visible subjects. This
            // bounds pair checks in particle scenes and avoids treating tiny details as leads.
            moving.truncate(8);
            let mut reading_actions = BTreeSet::new();
            for (text_node, _, _) in &visible_text {
                if tree
                    .node_texts
                    .get(text_node)
                    .is_some_and(|text| reading_words(text) >= 2)
                    && active_text.get(text_node).is_some_and(|run| {
                        f64::from(frame - run.start_frame) / crate::frame_rate_as_f64(fps)
                            < run.recommended_seconds
                    })
                {
                    for (action, _, _) in &moving {
                        reading_actions.insert((text_node.clone(), action.clone()));
                    }
                }
            }
            update_frame_runs(
                &mut active_reading_actions,
                &reading_actions,
                frame,
                |nodes, run| emit_motion_while_reading(nodes, run, &mut issues),
            );
            let mut simultaneous = BTreeSet::new();
            for (index, (first, _, first_bounds)) in moving.iter().enumerate() {
                for (second, _, second_bounds) in moving.iter().skip(index + 1) {
                    if separate_boxes(*first_bounds, *second_bounds) {
                        let pair = if first < second {
                            (first.clone(), second.clone())
                        } else {
                            (second.clone(), first.clone())
                        };
                        simultaneous.insert(pair);
                    }
                }
            }
            update_frame_runs(
                &mut active_simultaneous_actions,
                &simultaneous,
                frame,
                |nodes, run| emit_simultaneous_actions(nodes, run, &mut issues),
            );
            for node in candidates.iter().filter(|node| !text_nodes.contains(*node)) {
                let delta =
                    previous
                        .get(node)
                        .zip(current.get(node))
                        .and_then(|(before, after)| {
                            if !before.visible
                                || !before.on_screen
                                || before.motion_blurred
                                || !after.visible
                                || !after.on_screen
                                || after.motion_blurred
                            {
                                return None;
                            }
                            let displacement = [
                                after.position.x - before.position.x,
                                after.position.y - before.position.y,
                            ];
                            let distance =
                                valle_draw::math::hypot(displacement[0], displacement[1]);
                            let area = visible_area(after.bounds, viewport);
                            (distance.is_finite()
                                && (3.0..=jump_limit).contains(&distance)
                                && area.is_finite()
                                && area >= viewport[0] * viewport[1] * LINEAR_AREA_FRACTION)
                                .then_some(displacement)
                        });
                let previous_delta = last_delta.get(node).copied();
                match (previous_delta, delta) {
                    (Some(before), Some(after))
                        if valle_draw::math::hypot(after[0] - before[0], after[1] - before[1])
                            <= valle_draw::math::hypot(after[0], after[1]) * 0.02 + 0.2 =>
                    {
                        active_linear
                            .entry(node.clone())
                            .and_modify(|run| run.frames.end_frame = frame)
                            .or_insert(LinearRun {
                                frames: FrameRun {
                                    start_frame: frame - 2,
                                    end_frame: frame,
                                },
                                delta: after,
                            });
                    }
                    _ => {
                        if let Some(run) = active_linear.remove(node) {
                            emit_linear_motion(node, run, viewport, &mut issues);
                        }
                    }
                }
                if let Some(delta) = delta {
                    if previous_delta.is_none() {
                        motion_onsets.entry(node.clone()).or_insert(frame);
                    }
                    last_delta.insert(node.clone(), delta);
                } else {
                    last_delta.remove(node);
                }
            }
            for node in &candidates {
                let motion = previous.get(node).zip(current.get(node)).and_then(
                    |(before, after): (
                        &super::scene::ReviewNodeSample,
                        &super::scene::ReviewNodeSample,
                    )| {
                        if !before.visible
                            || !after.visible
                            || before.motion_blurred
                            || after.motion_blurred
                            || !before.on_screen
                            || !after.on_screen
                            || valle_draw::math::sqrt(
                                (after.bounds[2] - after.bounds[0]).max(0.0)
                                    * (after.bounds[3] - after.bounds[1]).max(0.0),
                            ) * after.opacity
                                < MIN_STROBE_EQUIVALENT_SIZE
                        {
                            return None;
                        }
                        let dx = after.position.x - before.position.x;
                        let dy = after.position.y - before.position.y;
                        let distance = valle_draw::math::hypot(dx, dy);
                        let width = (before.bounds[2] - before.bounds[0])
                            .max(after.bounds[2] - after.bounds[0]);
                        let height = (before.bounds[3] - before.bounds[1])
                            .max(after.bounds[3] - after.bounds[1]);
                        let threshold = if distance > 0.0 {
                            STROBE_EXTENT_FRACTION * (dx.abs() * width + dy.abs() * height)
                                / distance
                        } else {
                            0.0
                        };
                        (distance.is_finite()
                            && threshold.is_finite()
                            && threshold > 0.0
                            && distance > threshold
                            && distance <= jump_limit)
                            .then_some((distance, threshold))
                    },
                );
                if let Some((distance, threshold)) = motion {
                    let run = active.entry(node.clone()).or_insert(FastRun {
                        start_frame: frame - 1,
                        end_frame: frame,
                        peak: distance,
                        peak_threshold: threshold,
                    });
                    run.end_frame = frame;
                    if distance > run.peak {
                        run.peak = distance;
                        run.peak_threshold = threshold;
                    }
                } else if let Some(run) = active.remove(node) {
                    emit_run(node, run, &mut issues);
                }
            }
        }
        previous = current;
    }
    for (node, run) in active {
        emit_run(&node, run, &mut issues);
    }
    for (node, run) in active_text {
        emit_text_run(&node, run, fps, &mut issues);
    }
    for (node, run) in active_out_of_frame {
        emit_out_of_frame(&node, run, &mut issues);
    }
    for (nodes, run) in active_overlaps {
        emit_overlap(&nodes, run, &mut issues);
    }
    for (nodes, run) in active_reading_actions {
        emit_motion_while_reading(&nodes, run, &mut issues);
    }
    for (nodes, run) in active_simultaneous_actions {
        emit_simultaneous_actions(&nodes, run, &mut issues);
    }
    let viewport = opts
        .viewport
        .size
        .width
        .zip(opts.viewport.size.height)
        .map_or([f64::INFINITY; 2], |(width, height)| {
            [f64::from(width), f64::from(height)]
        });
    for (node, run) in active_linear {
        emit_linear_motion(&node, run, viewport, &mut issues);
    }
    emit_stagger_timing(&candidates, &motion_onsets, fps, &mut issues);
    merge_batch_strobes(&mut issues, &groups);
    issues.sort_by(|a, b| {
        a.node
            .cmp(&b.node)
            .then(a.start_frame.cmp(&b.start_frame))
            .then(a.code.cmp(b.code))
            .then(a.other_node.cmp(&b.other_node))
    });
    Ok(MotionReview {
        frames_analyzed: duration_frames,
        nodes_analyzed: candidates.len(),
        issues,
        trajectories: trajectories
            .into_iter()
            .map(|(node, points)| MotionTrajectory { node, points })
            .collect(),
    })
}
