//! Bounded, compile-local common subexpression reuse. Keys contain the complete lowered
//! expression, including ordered child IDs, seeds and scoped node/prop references. Nothing is
//! algebraically rewritten, and each instance arena is detached before its temporary IDs roll back.

use std::{collections::HashMap, io};

use valle_motion::{Expr, ExprId};

const MAX_KEY_BYTES: usize = 4 * 1024;
const MAX_ENTRIES: usize = 65_536;
// Account both copies of each key and a conservative per-entry bookkeeping allowance. This is
// a retained key budget, not a process memory bound; expressions/source maps have their own limits.
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;
const ENTRY_BOOKKEEPING_BYTES: usize = 128;

#[derive(Clone, Default)]
pub(super) struct ExprInterner {
    entries: HashMap<Vec<u8>, ExprId>,
    history: Vec<(ExprId, Vec<u8>, usize)>,
    retained_bytes: usize,
}

impl ExprInterner {
    /// Return an earlier identical expression, or remember this ID when capacity permits.
    /// Call only after type/finite-value validation; JSON then preserves all finite float bit
    /// patterns, including signed zero. Hash collisions still compare the complete key bytes.
    /// `clock` tags the seconds of one TimeScope clock: an equal expression on another clock,
    /// or one an author writes, is a different value.
    pub(super) fn intern(
        &mut self,
        expression: &Expr,
        clock: Option<u32>,
        id: ExprId,
    ) -> Option<ExprId> {
        let mut key = LimitedKey(Vec::new());
        serde_json::to_writer(&mut key, expression).ok()?;
        if let Some(clock) = clock {
            io::Write::write_all(&mut key, format!("@clock{clock}").as_bytes()).ok()?;
        }
        let key = key.0;
        if let Some(existing) = self.entries.get(&key) {
            return Some(*existing);
        }
        let cost = key.capacity() + key.len() + ENTRY_BOOKKEEPING_BYTES;
        if self.entries.len() < MAX_ENTRIES && self.retained_bytes + cost <= MAX_RETAINED_BYTES {
            self.history.push((id, key.clone(), cost));
            self.entries.insert(key, id);
            self.retained_bytes += cost;
        }
        None
    }

    /// Remove only the rolled-back suffix, avoiding a whole-table scan on every folded value.
    pub(super) fn truncate(&mut self, len: usize) {
        while self
            .history
            .last()
            .is_some_and(|(id, _, _)| id.0 as usize >= len)
        {
            let (_, key, cost) = self.history.pop().expect("checked suffix");
            self.entries.remove(&key);
            self.retained_bytes -= cost;
        }
    }
}

struct LimitedKey(Vec<u8>);

impl io::Write for LimitedKey {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_KEY_BYTES - self.0.len() {
            return Err(io::Error::other("expression interning key exceeds budget"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use valle_motion::MotionValue;

    fn number(value: f64) -> Expr {
        Expr::Const {
            value: MotionValue::Number(value),
        }
    }

    #[test]
    fn rollback_never_returns_an_id_from_a_discarded_instance_or_fold() {
        let mut pool = ExprInterner::default();
        assert_eq!(pool.intern(&number(1.0), None, ExprId(0)), None);
        assert_eq!(pool.intern(&number(2.0), None, ExprId(1)), None);
        assert_eq!(pool.intern(&number(3.0), None, ExprId(2)), None);
        pool.truncate(1);
        assert_eq!(pool.intern(&number(1.0), None, ExprId(1)), Some(ExprId(0)));
        assert_eq!(pool.intern(&number(3.0), None, ExprId(1)), None);
        assert_eq!(pool.intern(&number(3.0), None, ExprId(2)), Some(ExprId(1)));
        pool.truncate(0);
        assert_eq!(pool.retained_bytes, 0);
        assert!(pool.entries.is_empty());
    }

    #[test]
    fn equal_expressions_on_different_clocks_stay_apart() {
        let mut pool = ExprInterner::default();
        assert_eq!(pool.intern(&number(1.0), None, ExprId(0)), None);
        assert_eq!(pool.intern(&number(1.0), Some(0), ExprId(1)), None);
        assert_eq!(pool.intern(&number(1.0), Some(1), ExprId(2)), None);
        assert_eq!(pool.intern(&number(1.0), Some(0), ExprId(3)), Some(ExprId(1)));
        assert_eq!(pool.intern(&number(1.0), None, ExprId(3)), Some(ExprId(0)));
    }

    #[test]
    fn ordered_operands_and_float_bits_are_not_normalized() {
        let mut pool = ExprInterner::default();
        for (at, value) in [
            0.0,
            -0.0,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            1.0,
            f64::from_bits(1.0f64.to_bits() + 1),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(pool.intern(&number(value), None, ExprId(at as u32)), None);
            assert_eq!(
                pool.intern(&number(value), None, ExprId(99)),
                Some(ExprId(at as u32))
            );
        }
        let a = Expr::Add {
            lhs: ExprId(0),
            rhs: ExprId(1),
        };
        let b = Expr::Add {
            lhs: ExprId(1),
            rhs: ExprId(0),
        };
        assert_eq!(pool.intern(&a, None, ExprId(6)), None);
        assert_eq!(pool.intern(&b, None, ExprId(7)), None);
    }

    #[test]
    fn large_keys_and_full_capacity_bypass_without_dropping_valid_lookups() {
        let mut pool = ExprInterner::default();
        let large = Expr::Const {
            value: MotionValue::Str("x".repeat(MAX_KEY_BYTES)),
        };
        assert_eq!(pool.intern(&large, None, ExprId(0)), None);
        assert!(pool.entries.is_empty());
        for at in 0..100_000 {
            pool.intern(&number(f64::from(at)), None, ExprId(at));
        }
        assert!(pool.entries.len() <= MAX_ENTRIES);
        assert!(pool.retained_bytes <= MAX_RETAINED_BYTES);
        assert!(pool.entries.len() < 100_000);
        assert_eq!(
            pool.intern(&number(0.0), None, ExprId(100_000)),
            Some(ExprId(0))
        );
        pool.truncate(0);
        assert_eq!(pool.retained_bytes, 0);
    }
}
