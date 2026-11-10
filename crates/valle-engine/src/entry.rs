//! File names distinguish playable entries from ordinary modules and data.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Motion,
    Timeline,
}

impl Kind {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Motion => ".motion.tsx",
            Self::Timeline => ".timeline.json",
        }
    }

    pub fn read(path: &str) -> Result<Self, String> {
        [Self::Motion, Self::Timeline]
            .into_iter()
            .find(|kind| path.ends_with(kind.suffix()))
            .ok_or_else(|| format!("{path}: an entry must end with .motion.tsx or .timeline.json"))
    }

    pub fn check(self, path: &str) -> Result<(), String> {
        match path.ends_with(self.suffix()) {
            true => Ok(()),
            false => Err(format!("{path}: the entry must end with {}", self.suffix())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_have_compound_suffixes() {
        for (path, kind) in [
            ("scene.motion.tsx", Kind::Motion),
            ("components/title.motion.tsx", Kind::Motion),
            ("https://example.com/title.motion.tsx", Kind::Motion),
            ("C:\\film\\main.timeline.json", Kind::Timeline),
            ("/film/timelines/cut.timeline.json", Kind::Timeline),
        ] {
            assert_eq!(Kind::read(path), Ok(kind));
            kind.check(path).unwrap();
        }
        for path in [
            "Card.tsx",
            "data.json",
            "timeline.json",
            "scene.motion.tsx.bak",
        ] {
            assert!(Kind::read(path).is_err(), "{path}");
        }
        assert!(
            Kind::Motion
                .check("cut.timeline.json")
                .unwrap_err()
                .contains(".motion.tsx")
        );
        assert!(
            Kind::Timeline
                .check("scene.motion.tsx")
                .unwrap_err()
                .contains(".timeline.json")
        );
    }
}
