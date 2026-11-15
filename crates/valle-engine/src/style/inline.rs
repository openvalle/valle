//! CSS inline alignment. Percentages refer to this inline's computed line height.
use super::units::{Len, SPACING, length};
use crate::template::V;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Align<T> {
    #[default]
    Baseline,
    Middle,
    Sub,
    Super,
    TextTop,
    TextBottom,
    Top,
    Bottom,
    Offset(T),
}

impl<T> Align<T> {
    pub fn map<U>(&self, f: impl FnOnce(&T) -> U) -> Align<U> {
        match self {
            Self::Baseline => Align::Baseline,
            Self::Middle => Align::Middle,
            Self::Sub => Align::Sub,
            Self::Super => Align::Super,
            Self::TextTop => Align::TextTop,
            Self::TextBottom => Align::TextBottom,
            Self::Top => Align::Top,
            Self::Bottom => Align::Bottom,
            Self::Offset(value) => Align::Offset(f(value)),
        }
    }
}

pub(super) fn parse(v: &V) -> Result<Align<Len>, String> {
    Ok(match v.text().trim().to_ascii_lowercase().as_str() {
        "baseline" => Align::Baseline,
        "middle" => Align::Middle,
        "sub" => Align::Sub,
        "super" => Align::Super,
        "text-top" => Align::TextTop,
        "text-bottom" => Align::TextBottom,
        "top" => Align::Top,
        "bottom" => Align::Bottom,
        _ => Align::Offset(length(
            v,
            super::units::Accept {
                percent: true,
                keyword: None,
                ..SPACING
            },
        )?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::styles::{num, one, styles, text, view};

    #[test]
    fn inline_alignment_parses_units_and_inherits_only_explicitly() {
        for name in [
            "baseline",
            "middle",
            "sub",
            "super",
            "top",
            "bottom",
            "text-top",
            "text-bottom",
            "25%",
            "-1em",
            "calc(1px + 20%)",
        ] {
            assert!(parse(&V::Str(name.into())).is_ok(), "{name}");
        }
        for name in ["normal", "auto", "center", "1", "10deg"] {
            assert!(parse(&V::Str(name.into())).is_err(), "{name}");
        }
        let result = styles(
            vec![
                view(
                    &[("fontSize", num(20.0)), ("verticalAlign", text("1em"))],
                    &[1, 2],
                ),
                view(&[("fontSize", num(30.0))], &[]),
                view(
                    &[("fontSize", num(30.0)), ("verticalAlign", text("inherit"))],
                    &[],
                ),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(result[1].vertical_align, Align::Baseline);
        let Align::Offset(v) = &result[2].vertical_align else {
            panic!("a length")
        };
        assert_eq!(v.resolve(100.0), 20.0);
        assert!(!one(&[]).lays_out_as(&one(&[("verticalAlign", text("super"))])));
    }
}
