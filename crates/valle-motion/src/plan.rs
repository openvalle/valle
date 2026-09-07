//! Deterministic prepare-time dependency plan for cross-clip Motion signals.
//!
//! Cue windows are precomputed/random-access inputs and therefore create no edges. Future
//! post-layout providers populate `providers`; this module already freezes ordering,
//! missing-provider, self-reference, and cycle behavior without coupling the plan to a renderer
//! or paint order.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{DiagCode, MotionDiagnostic, PlanError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanClip {
    pub clip_id: String,
    pub start_frame: i64,
    /// Clip ids whose layout-derived signals must exist before this clip can evaluate.
    #[serde(default)]
    pub providers: Vec<String>,
}

impl PlanClip {
    pub fn new(clip_id: impl Into<String>, start_frame: i64) -> Self {
        Self {
            clip_id: clip_id.into(),
            start_frame,
            providers: Vec::new(),
        }
    }

    pub fn with_providers(
        mut self,
        providers: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.providers = providers.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanStep {
    pub clip_id: String,
    pub rank: u32,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignalPlan {
    pub steps: Vec<PlanStep>,
}

impl SignalPlan {
    /// Stable Kahn topological sort. Ready nodes are ordered by `(start_frame, clip_id)`, so the
    /// serialized plan does not depend on caller iteration order.
    pub fn build(clips: &[PlanClip]) -> Result<Self, PlanError> {
        let mut by_id = BTreeMap::<&str, &PlanClip>::new();
        for clip in clips {
            if clip.clip_id.is_empty() {
                return Err(MotionDiagnostic::new(
                    DiagCode::DuplicateClip,
                    "clip:",
                    "clip id must not be empty",
                ));
            }
            if by_id.insert(&clip.clip_id, clip).is_some() {
                return Err(MotionDiagnostic::new(
                    DiagCode::DuplicateClip,
                    format!("clip:{}", clip.clip_id),
                    format!("clip id `{}` appears twice in the plan", clip.clip_id),
                ));
            }
        }

        let key = |clip: &PlanClip| (clip.start_frame, clip.clip_id.clone());
        let mut dependencies = BTreeMap::<&str, BTreeSet<&str>>::new();
        let mut dependents = BTreeMap::<&str, BTreeSet<&str>>::new();
        for clip in clips {
            let mut mine = BTreeSet::new();
            for provider in &clip.providers {
                if provider == &clip.clip_id {
                    return Err(MotionDiagnostic::new(
                        DiagCode::SignalSelfReference,
                        format!("clip:{}", clip.clip_id),
                        format!("clip `{}` depends on its own layout", clip.clip_id),
                    ));
                }
                if !by_id.contains_key(provider.as_str()) {
                    return Err(MotionDiagnostic::new(
                        DiagCode::SignalProviderMissing,
                        format!("clip:{}/provider:{}", clip.clip_id, provider),
                        format!("provider clip `{provider}` is not in this plan"),
                    ));
                }
                mine.insert(provider.as_str());
                dependents
                    .entry(provider)
                    .or_default()
                    .insert(&clip.clip_id);
            }
            dependencies.insert(&clip.clip_id, mine);
        }

        let mut indegree = dependencies
            .iter()
            .map(|(id, providers)| (*id, providers.len()))
            .collect::<BTreeMap<_, _>>();
        let mut ready = clips
            .iter()
            .filter(|clip| indegree[clip.clip_id.as_str()] == 0)
            .map(key)
            .collect::<BTreeSet<_>>();
        let mut rank_of = BTreeMap::<&str, u32>::new();
        let mut steps = Vec::with_capacity(clips.len());

        while let Some(next) = ready.pop_first() {
            let id = next.1;
            let providers = &dependencies[id.as_str()];
            let rank = providers
                .iter()
                .map(|provider| rank_of[provider] + 1)
                .max()
                .unwrap_or(0);
            let clip = by_id[id.as_str()];
            rank_of.insert(&clip.clip_id, rank);
            steps.push(PlanStep {
                clip_id: id.clone(),
                rank,
                depends_on: providers.iter().map(|value| (*value).to_owned()).collect(),
            });
            for dependent in dependents.get(id.as_str()).into_iter().flatten() {
                let degree = indegree
                    .get_mut(*dependent)
                    .expect("dependent was admitted above");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(key(by_id[*dependent]));
                }
            }
        }

        if steps.len() != clips.len() {
            let mut stuck = clips
                .iter()
                .filter(|clip| !rank_of.contains_key(clip.clip_id.as_str()))
                .collect::<Vec<_>>();
            stuck.sort_by_key(|clip| key(clip));
            let names = stuck
                .iter()
                .map(|clip| clip.clip_id.as_str())
                .collect::<Vec<_>>();
            return Err(MotionDiagnostic::new(
                DiagCode::SignalCycle,
                format!("clip:{}", names[0]),
                format!("signal dependency cycle among: {}", names.join(", ")),
            ));
        }

        Ok(Self { steps })
    }

    pub fn step(&self, clip_id: &str) -> Option<&PlanStep> {
        self.steps.iter().find(|step| step.clip_id == clip_id)
    }

    pub fn passes(&self) -> Vec<Vec<&PlanStep>> {
        let mut passes = Vec::<Vec<&PlanStep>>::new();
        for step in &self.steps {
            while passes.len() <= step.rank as usize {
                passes.push(Vec::new());
            }
            passes[step.rank as usize].push(step);
        }
        passes
    }

    pub fn depth(&self) -> u32 {
        self.steps
            .iter()
            .map(|step| step.rank + 1)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_topological_order_and_ranks() {
        let clips = vec![
            PlanClip::new("c", 30).with_providers(["b"]),
            PlanClip::new("b", 20).with_providers(["a"]),
            PlanClip::new("a", 10),
            PlanClip::new("z", 0),
        ];
        let plan = SignalPlan::build(&clips).unwrap();
        assert_eq!(
            plan.steps
                .iter()
                .map(|step| step.clip_id.as_str())
                .collect::<Vec<_>>(),
            ["z", "a", "b", "c"]
        );
        assert_eq!(plan.step("c").unwrap().rank, 2);
        assert_eq!(plan.depth(), 3);

        let shuffled = clips.into_iter().rev().collect::<Vec<_>>();
        assert_eq!(SignalPlan::build(&shuffled).unwrap(), plan);
    }

    #[test]
    fn missing_self_duplicate_and_cycles_fail_closed() {
        assert_eq!(
            SignalPlan::build(&[PlanClip::new("a", 0).with_providers(["missing"])])
                .unwrap_err()
                .code,
            DiagCode::SignalProviderMissing
        );
        assert_eq!(
            SignalPlan::build(&[PlanClip::new("a", 0).with_providers(["a"])])
                .unwrap_err()
                .code,
            DiagCode::SignalSelfReference
        );
        assert_eq!(
            SignalPlan::build(&[PlanClip::new("a", 0), PlanClip::new("a", 1)])
                .unwrap_err()
                .code,
            DiagCode::DuplicateClip
        );
        assert_eq!(
            SignalPlan::build(&[
                PlanClip::new("a", 0).with_providers(["b"]),
                PlanClip::new("b", 1).with_providers(["a"]),
            ])
            .unwrap_err()
            .code,
            DiagCode::SignalCycle
        );
    }

    #[test]
    fn independent_inputs_are_one_pass_and_json_is_stable() {
        let plan = SignalPlan::build(&[
            PlanClip::new("b", 0),
            PlanClip::new("a", 0),
            PlanClip::new("c", 2),
        ])
        .unwrap();
        assert_eq!(plan.depth(), 1);
        assert_eq!(plan.passes().len(), 1);
        assert_eq!(
            serde_json::to_string(&plan).unwrap(),
            r#"{"steps":[{"clipId":"a","rank":0,"dependsOn":[]},{"clipId":"b","rank":0,"dependsOn":[]},{"clipId":"c","rank":0,"dependsOn":[]}]}"#
        );
    }
}
