//! Immutable paint table with sparse frame bindings. Static gradients and unused colors stay
//! shared; binding one color does not copy a whole scene's paint arena.

use super::Paint;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PaintTable {
    base: Arc<[Paint]>,
    replacements: Arc<[(usize, Paint)]>,
}

impl From<Vec<Paint>> for PaintTable {
    fn from(base: Vec<Paint>) -> Self {
        Self {
            base: base.into(),
            replacements: Arc::from([]),
        }
    }
}

impl PaintTable {
    pub fn len(&self) -> usize {
        self.base.len()
    }
    pub fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Paint> {
        match self
            .replacements
            .binary_search_by_key(&index, |(index, _)| *index)
        {
            Ok(found) => Some(&self.replacements[found].1),
            Err(_) => self.base.get(index),
        }
    }

    pub fn iter(&self) -> PaintIter<'_> {
        PaintIter {
            table: self,
            front: 0,
            back: self.len(),
        }
    }

    pub fn to_vec(&self) -> Vec<Paint> {
        self.iter().cloned().collect()
    }

    pub fn contains(&self, paint: &Paint) -> bool {
        self.iter().any(|candidate| candidate == paint)
    }

    pub(super) fn replaced(&self, replacements: impl Iterator<Item = (usize, Paint)>) -> Self {
        // A template may itself contain immutable bindings. Merge them once without retaining
        // a chain of previous-frame overlays; lookup depth and storage never depend on history.
        let mut merged: std::collections::BTreeMap<_, _> =
            self.replacements.iter().cloned().collect();
        merged.extend(replacements);
        Self {
            base: self.base.clone(),
            replacements: merged.into_iter().collect(),
        }
    }
}

impl std::ops::Index<usize> for PaintTable {
    type Output = Paint;
    fn index(&self, index: usize) -> &Paint {
        self.get(index).expect("paint index out of bounds")
    }
}

impl PartialEq for PaintTable {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl serde::Serialize for PaintTable {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

pub struct PaintIter<'a> {
    table: &'a PaintTable,
    front: usize,
    back: usize,
}

impl<'a> Iterator for PaintIter<'a> {
    type Item = &'a Paint;
    fn next(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        let index = self.front;
        self.front += 1;
        self.table.get(index)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.back - self.front;
        (remaining, Some(remaining))
    }
}
impl DoubleEndedIterator for PaintIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.table.get(self.back)
    }
}
impl ExactSizeIterator for PaintIter<'_> {}
impl std::iter::FusedIterator for PaintIter<'_> {}
impl<'a> IntoIterator for &'a PaintTable {
    type Item = &'a Paint;
    type IntoIter = PaintIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::{GradientStop, LinearColor, SpreadMode};

    #[test]
    fn sparse_binding_shares_static_gradients_and_does_not_chain_frame_history() {
        let red = Paint::Solid(LinearColor::new(1., 0., 0., 1.));
        let blue = Paint::Solid(LinearColor::new(0., 0., 1., 1.));
        let gradient = Paint::LinearGradient {
            start: [0., 0.],
            end: [100., 100.],
            spread: SpreadMode::Pad,
            stops: vec![GradientStop {
                offset: 0.,
                color: LinearColor::new(4., -0.1, 0.5, 1.),
            }],
        };
        let table = PaintTable::from(vec![red.clone(), gradient.clone(), blue.clone()]);
        let bound = table.replaced([(0, blue.clone())].into_iter());
        assert!(Arc::ptr_eq(&table.base, &bound.base));
        assert!(std::ptr::eq(table.get(1).unwrap(), bound.get(1).unwrap()));
        assert_eq!(table[0], red);
        assert_eq!(bound[0], blue);
        assert_eq!(
            serde_json::to_vec(&bound).unwrap(),
            serde_json::to_vec(&vec![blue, gradient, table[2].clone()]).unwrap()
        );
        let rebound = bound.replaced([(0, red.clone()), (2, red)].into_iter());
        assert!(Arc::ptr_eq(&table.base, &rebound.base));
        assert_eq!(rebound.replacements.len(), 2);
        assert_eq!(rebound.iter().rev().count(), 3);
        assert_eq!(rebound.iter().nth(1), table.get(1));
        assert!(rebound.get(3).is_none());
    }
}
