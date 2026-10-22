//! Which clock a value reads. A TimeScope gives the components drawn inside it their own scene
//! clock, `(outer seconds - offset) × speed`; the clip clock and `useComposition()` hold
//! everywhere. A node draws values of its own scene clock only.

use super::*;

/// What an expression reads of time.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) struct TimeReads {
    /// Computed from the clock: the scene or clip seconds, progress, or the frame. Comparisons,
    /// rounding and remainders on these take the clock tolerance.
    clock: bool,
    /// Computed from the start or end of a caption run, a time on the clip clock.
    run_time: bool,
    /// The scene seconds, possibly offset by constants: a time that constants of seconds meet.
    scene_seconds: bool,
}

/// Whether `input` changes with the clock: the scene or clip seconds, progress, or the frame.
pub(super) fn reads_clock(input: ContextInput) -> bool {
    reads_scene_clock(input)
        || matches!(
            input,
            ContextInput::HostSeconds | ContextInput::HostProgress
        )
}

/// Whether `input` is the composition's scene clock, which a TimeScope changes.
pub(super) fn reads_scene_clock(input: ContextInput) -> bool {
    matches!(
        input,
        ContextInput::CompositionSeconds | ContextInput::LocalFrame | ContextInput::LocalProgress
    )
}

impl<'s> Compiler<'s> {
    fn time_reads(&self, expr: ExprId) -> TimeReads {
        self.expr_arena
            .time_reads
            .get(expr.0 as usize)
            .copied()
            .unwrap_or_default()
    }

    /// What `expression` reads of time, from what its children read. `clock` is set on the
    /// seconds of a TimeScope's clock.
    pub(super) fn time_reads_of(&self, expression: &Expr, clock: Option<ClockId>) -> TimeReads {
        let mut reads = TimeReads::default();
        for child in expression.children() {
            let child = self.time_reads(child);
            reads.clock |= child.clock;
            reads.run_time |= child.run_time;
        }
        let constant = |expr: &ExprId| {
            matches!(
                self.expr_arena.values.get(expr.0 as usize),
                Some(Expr::Const {
                    value: MotionValue::Number(_)
                })
            )
        };
        match expression {
            Expr::Context { input } => {
                reads.clock |= reads_clock(*input);
                reads.scene_seconds = *input == ContextInput::CompositionSeconds;
            }
            Expr::Caption {
                input: valle_motion::CaptionInput::RunStart | valle_motion::CaptionInput::RunEnd,
            } => reads.run_time = true,
            Expr::Add { lhs, rhs } => {
                reads.scene_seconds = (self.time_reads(*lhs).scene_seconds && constant(rhs))
                    || (constant(lhs) && self.time_reads(*rhs).scene_seconds);
            }
            Expr::Sub { lhs, rhs } => {
                reads.scene_seconds = self.time_reads(*lhs).scene_seconds && constant(rhs);
            }
            _ => {}
        }
        // Only the seconds of a TimeScope's clock are pushed on it.
        reads.scene_seconds |= clock.is_some();
        reads
    }

    /// Whether `expr` is computed from the clock.
    pub(super) fn reads_clock(&self, expr: ExprId) -> bool {
        self.time_reads(expr).clock
    }

    /// A comparison of two values, with the clock tolerance when one is computed from the clock.
    /// Rejects times that cannot meet as written, and warns of a constant past the end of the
    /// scene that is likely not in seconds.
    pub(super) fn compare(
        &mut self,
        op: CompareOp,
        lhs: ExprId,
        rhs: ExprId,
        span: Span,
    ) -> ExprId {
        self.check_time_pair(lhs, rhs, span);
        let clip = |compiler: &Self, expr: ExprId, input: ContextInput| {
            matches!(
                compiler.expr_arena.values.get(expr.0 as usize),
                Some(Expr::Context { input: found }) if *found == input
            )
        };
        if (clip(self, lhs, ContextInput::HostSeconds)
            && clip(self, rhs, ContextInput::HostDuration))
            || (clip(self, lhs, ContextInput::HostDuration)
                && clip(self, rhs, ContextInput::HostSeconds))
        {
            self.illegal(DiagCode::GrammarForbidden, span, CLIP_END_MESSAGE);
        }
        for (time, constant) in [(lhs, rhs), (rhs, lhs)] {
            if self.time_reads(time).scene_seconds
                && let Some(Expr::Const {
                    value: MotionValue::Number(value),
                }) = self.expr_arena.values.get(constant.0 as usize)
            {
                self.check_time_unit(*value, span);
            }
        }
        let op = self.compare_op(op, lhs, rhs);
        self.push(Expr::Compare { op, lhs, rhs }, span)
    }

    /// Reject a caption run's time met with the scene time: the run is timed on the clip clock.
    pub(super) fn check_time_pair(&mut self, lhs: ExprId, rhs: ExprId, span: Span) {
        let scene = |compiler: &Self, expr: ExprId| {
            !matches!(
                compiler.expr_arena.scene_clock.get(expr.0 as usize),
                None | Some(SceneClock::None)
            )
        };
        if (self.time_reads(lhs).run_time && scene(self, rhs))
            || (scene(self, lhs) && self.time_reads(rhs).run_time)
        {
            self.illegal(
                DiagCode::GrammarForbidden,
                span,
                "the `start` and `end` of a caption run are times of the clip, but this meets one \
                 with the scene time; meet them with `clipSeconds` from `useCurrentTime()`, such \
                 as `clipSeconds >= run.start`",
            );
        }
    }

    /// Reject an interpolation of `clipSeconds` that ends at `clipDuration`, which the last frame
    /// never reaches, and warn of a stop past the end of the scene in an interpolation of the
    /// scene seconds.
    pub(super) fn check_interpolate_times(
        &mut self,
        time: ExprId,
        input: ExprId,
        stops: &[f64],
        span: Span,
    ) {
        if let Some(Expr::Sub { lhs, rhs }) = self.expr_arena.values.get(input.0 as usize)
            && matches!(
                self.expr_arena.values.get(lhs.0 as usize),
                Some(Expr::Context {
                    input: ContextInput::HostSeconds
                })
            )
            && matches!(
                self.expr_arena.values.get(rhs.0 as usize),
                Some(Expr::Context {
                    input: ContextInput::HostDuration
                })
            )
            && stops.last() == Some(&0.0)
        {
            self.illegal(DiagCode::GrammarForbidden, span, CLIP_END_MESSAGE);
        }
        if self.time_reads(time).scene_seconds
            && let Some(last) = stops.iter().copied().reduce(f64::max)
        {
            self.check_time_unit(last, span);
        }
    }

    /// Warn of an Echo whose copies reach back past the length of the scene: its delay is likely
    /// in milliseconds or frames.
    pub(super) fn check_echo_span(&mut self, count: u8, delay: f64, span: Span) {
        let Some(duration) = self
            .composition
            .as_ref()
            .and_then(|composition| composition.duration().ok())
        else {
            return;
        };
        let duration = duration.as_f64();
        let reach = delay * f64::from(count);
        if reach > duration {
            self.warn(
                DiagCode::TimeUnit,
                span,
                format!(
                    "Echo draws {count} copies `delay` seconds apart, which reach back {} seconds, \
                     longer than the scene, which lasts {} seconds; `delay` is in seconds, so {} \
                     milliseconds are `{}`",
                    seconds_text(reach),
                    seconds_text(duration),
                    seconds_text(delay),
                    seconds_text(delay / 1000.0),
                ),
            );
        }
    }

    /// Warn of a time of `value` seconds met with the scene time when it is past the end of the
    /// scene: it is likely in milliseconds or frames.
    pub(super) fn check_time_unit(&mut self, value: f64, span: Span) {
        let Some(composition) = self.composition.as_ref() else {
            return;
        };
        let Ok(duration) = composition.duration() else {
            return;
        };
        let duration = duration.as_f64();
        if !(value > duration) {
            return;
        }
        let mut message = format!(
            "`{}` seconds is past the end of the scene, which lasts {} seconds; times are in \
             seconds, so {} milliseconds are `{}`",
            seconds_text(value),
            seconds_text(duration),
            seconds_text(value),
            seconds_text(value / 1000.0),
        );
        if let Ok(Some(rate)) = composition.frame_rate() {
            let fps = rate.numerator() as f64 / rate.denominator() as f64;
            message.push_str(&format!(
                ", and {} frames at {} fps are `{}`",
                seconds_text(value),
                seconds_text(fps),
                seconds_text(value / fps)
            ));
        }
        self.warn(DiagCode::TimeUnit, span, message);
    }

    /// `op` with the clock tolerance when an operand is computed from the clock, so a switch
    /// written in seconds flips exactly on its frame. Other values compare as in JavaScript.
    pub(super) fn compare_op(&self, op: CompareOp, lhs: ExprId, rhs: ExprId) -> CompareOp {
        if self.reads_clock(lhs) || self.reads_clock(rhs) {
            op.on_clock()
        } else {
            op
        }
    }

    /// The clock of the TimeScopes around the node being lowered.
    pub(super) fn position_clock(&mut self) -> ClockId {
        if let Some(clock) = self.known_clock(&self.time_scopes) {
            return clock;
        }
        self.clock_chains.push(self.time_scopes.clone());
        ClockId(self.clock_chains.len() as u32 - 1)
    }

    fn known_clock(&self, chain: &[TimeTransform]) -> Option<ClockId> {
        self.clock_chains
            .iter()
            .position(|known| known.as_slice() == chain)
            .map(|at| ClockId(at as u32))
    }

    /// Whether `expr` can be drawn by a node on `clock`: it reads no scene clock, or that one.
    pub(super) fn holds_on_clock(&self, expr: ExprId, clock: Option<ClockId>) -> bool {
        match self.expr_arena.scene_clock.get(expr.0 as usize) {
            None | Some(SceneClock::None) => true,
            Some(SceneClock::On(on)) => Some(*on) == clock,
            Some(SceneClock::Mixed) => false,
        }
    }

    /// Reject a node that draws a scene-time value of another clock: one an outer component read,
    /// drawn inside a TimeScope, or one read inside a TimeScope, drawn outside it.
    pub(super) fn check_scene_clocks(
        &mut self,
        nodes: &[SceneNode],
        node_children: &[NodeId],
        root: NodeId,
    ) {
        let mut pending = vec![(root, Vec::new())];
        while let Some((id, chain)) = pending.pop() {
            let Some(node) = nodes.get(id.0 as usize) else {
                continue;
            };
            let clock = self.known_clock(&chain);
            let misplaced = node
                .expr_refs_outside_per_unit()
                .into_iter()
                .chain(node.per_unit_expr_refs())
                .any(|(_, expr)| !self.holds_on_clock(expr, clock));
            if misplaced {
                let span = self
                    .source_ledger
                    .node_spans
                    .get(id.0 as usize)
                    .copied()
                    .unwrap_or_else(|| Span::new(0, 0));
                self.illegal(
                    DiagCode::GrammarForbidden,
                    span,
                    scene_clock_message(!chain.is_empty()),
                );
            }
            let mut inner = chain;
            if let NodeKind::TimeScope {
                offset_seconds,
                speed,
            } = node.kind
            {
                inner.push(TimeTransform {
                    offset_seconds,
                    speed,
                });
            }
            let children = node_children
                .get(node.children.start as usize..node.children.end as usize)
                .unwrap_or_default();
            for child in children.iter().rev() {
                pending.push((*child, inner.clone()));
            }
        }
    }
}

/// Why `clipSeconds` cannot meet `clipDuration`, and what to write instead.
const CLIP_END_MESSAGE: &str = "`clipSeconds` never reaches `clipDuration`: the clip clock leaves \
     out the end of the clip, as a timeline does, so the last frame is drawn before it; animate to \
     the end of the clip with `clipProgress`, which is 1 on the last frame, such as \
     `interpolate(clipProgress, [0.9, 1], [1, 0])`";

/// A number of seconds as an author writes it: at most six decimals, without trailing zeros.
pub(super) fn seconds_text(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// Why a node cannot draw a scene-time value of another clock, and what to write instead.
pub(super) fn scene_clock_message(inside_time_scope: bool) -> &'static str {
    if inside_time_scope {
        "this node is drawn inside a TimeScope, which gives the components inside it their own \
         clock, but it uses the scene time of an outer component; read the time with \
         `useCurrentTime()` in a child component drawn inside the TimeScope, or move the node \
         outside it (TimeScope has no layout box, so the layout stays). JSX bound to a name or \
         pushed onto a list belongs to the component that writes it"
    } else {
        "this node uses scene time read inside a TimeScope, but it is drawn outside it; read the \
         time with `useCurrentTime()` in the component that draws the node"
    }
}
