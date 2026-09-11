//! Exact rational clocks shared by media backends. No native-library initialization is required.
#[derive(Debug, Clone, Copy, Eq)]
pub struct TimeBase(pub i32, pub i32);
impl TimeBase {
    pub const fn numerator(self) -> i32 {
        self.0
    }
    pub const fn denominator(self) -> i32 {
        self.1
    }
}

impl From<TimeBase> for f64 {
    fn from(value: TimeBase) -> Self {
        value.0 as f64 / value.1 as f64
    }
}

// Compare values without calling libav or changing the authored numerator and denominator.
impl PartialEq for TimeBase {
    fn eq(&self, other: &Self) -> bool {
        if self.1 == 0 || other.1 == 0 {
            return self.1 == other.1 && self.0.signum() == other.0.signum();
        }
        i64::from(self.0) * i64::from(other.1) == i64::from(other.0) * i64::from(self.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equivalent_clocks_preserve_authored_terms() {
        let clock = TimeBase(2, 60);
        assert_eq!(clock, TimeBase(1, 30));
        assert_eq!((clock.numerator(), clock.denominator()), (2, 60));
        assert_eq!(TimeBase(1, -30), TimeBase(-1, 30));
        assert_ne!(clock, TimeBase(1, 24));
    }
    #[test]
    fn extreme_terms_and_special_values_do_not_overflow() {
        assert_eq!(TimeBase(i32::MIN, i32::MIN), TimeBase(1, 1));
        assert_eq!(TimeBase(i32::MAX, 0), TimeBase(1, 0));
        assert_ne!(TimeBase(-1, 0), TimeBase(1, 0));
        assert_eq!(TimeBase(0, 0), TimeBase(0, 0));
        assert_ne!(TimeBase(0, 0), TimeBase(0, 1));
        assert_eq!(f64::from(TimeBase(1001, 30000)), 1001.0 / 30000.0);
    }
}
