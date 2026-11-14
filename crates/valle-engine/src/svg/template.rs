//! Fixed SVG string structure, with whole numeric expressions filled in each frame.
//! Keep the original pieces for inputs whose type or numeric spelling needs normal parsing.

use super::{Element, Shape};
use crate::path::{self, Path, Segment};
use crate::template::{Part, Tag, V};

pub(crate) struct Template {
    pub parts: Vec<Part>,
    tag: Tag,
    plan: Option<Plan>,
}

struct Plan {
    coordinates: Vec<Coordinate>,
    /// A move or a line for each point of an absolute M/L path; none for `points`.
    moves: Option<Vec<bool>>,
}

enum Coordinate {
    Fixed(f32),
    Expr(u32),
}

enum Token {
    Command(u8),
    Coordinate(Coordinate),
}

impl Template {
    pub fn new(tag: Tag, parts: Vec<Part>) -> Self {
        Self {
            tag,
            plan: Plan::new(tag, &parts),
            parts,
        }
    }

    pub fn resolve(&self, values: &[V]) -> Result<Shape, String> {
        if let Some(shape) = self.plan.as_ref().and_then(|p| p.resolve(self.tag, values)) {
            return Ok(shape);
        }
        let text = V::Str(crate::eval::template(&self.parts, values));
        let attribute = if self.tag == Tag::Path { "d" } else { "points" };
        let Element::Shape(shape) = super::computed(self.tag, &[(attribute, &text)], None)?.element
        else {
            unreachable!("only paths, polylines and polygons have string geometry")
        };
        Ok(shape)
    }
}

impl Plan {
    fn new(tag: Tag, parts: &[Part]) -> Option<Self> {
        let mut text = String::new();
        let mut expressions = Vec::new();
        for p in parts {
            match p {
                Part::Text(s) if !s.contains('\0') => text.push_str(s),
                Part::Expr(e) => {
                    text.push('\0');
                    expressions.push(*e);
                }
                _ => return None,
            }
        }
        let b = text.as_bytes();
        let space = |c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c');
        let boundary = |c| space(c) || matches!(c, b',' | b'M' | b'L');
        let mut tokens = Vec::new();
        let (mut i, mut expression) = (0, 0);
        while i < b.len() {
            if space(b[i]) {
                i += 1;
                continue;
            }
            match b[i] {
                c @ (b'M' | b'L') => {
                    tokens.push(Token::Command(c));
                    i += 1;
                }
                0 => {
                    // `${x}${y}`, `1${x}`, `${x}e2` and `-${x}` concatenate text,
                    // not whole coordinates. Never infer their meaning from one frame.
                    if i > 0 && !boundary(b[i - 1]) || i + 1 < b.len() && !boundary(b[i + 1]) {
                        return None;
                    }
                    tokens.push(Token::Coordinate(Coordinate::Expr(expressions[expression])));
                    expression += 1;
                    i += 1;
                    while i < b.len() && space(b[i]) {
                        i += 1;
                    }
                    if b.get(i) == Some(&b',') {
                        i += 1;
                    }
                }
                _ => {
                    let (n, used) = path::number_prefix(&text[i..])?;
                    tokens.push(Token::Coordinate(Coordinate::Fixed(n)));
                    i += used;
                }
            }
        }
        let mut coordinates = Vec::new();
        let mut moves = (tag == Tag::Path).then(Vec::new);
        let mut tokens = tokens.into_iter().peekable();
        let mut command = None;
        while tokens.peek().is_some() {
            if let Some(moves) = &mut moves {
                if let Some(Token::Command(_)) = tokens.peek() {
                    let Some(Token::Command(c)) = tokens.next() else {
                        unreachable!()
                    };
                    command = Some(c);
                }
                let c = command?;
                if moves.is_empty() && c != b'M' {
                    return None;
                }
                moves.push(c == b'M');
                command = Some(b'L'); // repeated pairs after M are lines
            }
            for _ in 0..2 {
                let Token::Coordinate(p) = tokens.next()? else {
                    return None;
                };
                coordinates.push(p);
            }
        }
        Some(Self { coordinates, moves })
    }

    fn resolve(&self, tag: Tag, values: &[V]) -> Option<Shape> {
        let coordinate = |p: &Coordinate| match p {
            Coordinate::Fixed(n) => Some(*n),
            Coordinate::Expr(e) => number(&values[*e as usize]),
        };
        let points = self
            .coordinates
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| Some([coordinate(&p[0])?, coordinate(&p[1])?]));
        match &self.moves {
            Some(moves) => {
                let segments = points
                    .zip(moves)
                    .map(|(p, &mov)| {
                        Some(if mov {
                            Segment::Move(p?)
                        } else {
                            Segment::Line(p?)
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(Shape::Path {
                    path: Path(segments),
                    ends: vec![true; moves.len()],
                })
            }
            None => Some(Shape::Points {
                points: points.collect::<Option<_>>()?,
                closed: tag == Tag::Polygon,
            }),
        }
    }
}

/// SVG parses the JS decimal spelling into f32. A direct cast agrees except at exact
/// midpoints, where that decimal can lie on the other side. Keep those on the original path.
fn number(v: &V) -> Option<f32> {
    let V::Num(n) = v else {
        return None;
    };
    let f = *n as f32;
    if !f.is_finite() {
        return None;
    }
    if *n == 0.0 {
        return Some(0.0);
    } // JS prints -0 as "0".
    let rounded = f64::from(f);
    if rounded != *n {
        let neighbor = if rounded < *n {
            f.next_up()
        } else {
            f.next_down()
        };
        if *n == (rounded + f64::from(neighbor)) * 0.5 {
            return None;
        }
    }
    Some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(text: &[&str]) -> Vec<Part> {
        let mut out = Vec::new();
        for (i, s) in text.iter().enumerate() {
            if i > 0 {
                out.push(Part::Expr((i - 1) as u32));
            }
            out.push(Part::Text((*s).into()));
        }
        out
    }

    fn parsed(tag: Tag, parts: &[Part], values: &[V]) -> Result<Shape, String> {
        let text = V::Str(crate::eval::template(parts, values));
        let attribute = if tag == Tag::Path { "d" } else { "points" };
        let Element::Shape(shape) =
            super::super::computed(tag, &[(attribute, &text)], None)?.element
        else {
            unreachable!()
        };
        Ok(shape)
    }

    #[test]
    fn fixed_structure_keeps_commands_markers_and_decimal_conversion() {
        for (tag, text) in [
            (Tag::Polyline, vec!["", ",", " 1e2,-.5"]),
            (Tag::Polygon, vec!["0,0 ", "\n", ",100,200"]),
            (Tag::Path, vec!["M", ",", "L100,200"]),
            (Tag::Path, vec!["M1,2 ", ",", " M3,4 L-5,6"]),
        ] {
            let t = Template::new(tag, parts(&text));
            assert!(t.plan.is_some(), "{text:?}");
            let mut numbers = vec![
                0.0,
                -0.0,
                1.0,
                -1.5,
                1e-45,
                -1e-45,
                1e-6,
                1e21,
                f64::from(f32::MAX),
            ];
            // Midpoints whose shortest decimal spelling can round to the other f32.
            for f in [1.0f32, 0.0, f32::MIN_POSITIVE, 12345.0] {
                let mid = (f64::from(f) + f64::from(f.next_up())) * 0.5;
                numbers.extend([mid, -mid, mid.next_up(), mid.next_down()]);
            }
            let mut bits = 7u64;
            for _ in 0..1000 {
                bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
                numbers.push(f64::from_bits(bits));
            }
            numbers.extend([f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e40]);
            for n in numbers {
                let values = [V::Num(n), V::Num(17.25)];
                assert_eq!(
                    t.resolve(&values),
                    parsed(tag, &t.parts, &values),
                    "{text:?}, {n:?}"
                );
            }
        }
        assert_eq!(number(&V::Num(-0.0)).unwrap().to_bits(), 0);
        assert_eq!(
            number(&V::Num(-1e-50)).unwrap().to_bits(),
            (-0.0f32).to_bits()
        );
    }

    #[test]
    fn concatenated_numbers_and_other_path_commands_keep_normal_parsing() {
        for (tag, text) in [
            (Tag::Polyline, vec!["1", ",20"]),
            (Tag::Polyline, vec!["", "e2,20"]),
            (Tag::Polyline, vec!["-", ",20"]),
            (Tag::Polyline, vec!["", "", ",20"]),
            (Tag::Polyline, vec!["", ",,20"]),
            (Tag::Path, vec!["M0,0 C", ",10 20,30 40,50"]),
            (Tag::Path, vec!["M0,0 l", ",20z"]),
        ] {
            let t = Template::new(tag, parts(&text));
            assert!(t.plan.is_none(), "{text:?}");
            let values = [V::Num(12.5), V::Num(3.0)];
            assert_eq!(
                t.resolve(&values),
                parsed(tag, &t.parts, &values),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_whole_number_can_later_be_text_without_freezing_structure_or_errors() {
        for (tag, text) in [
            (Tag::Polyline, vec!["", ",20"]),
            (Tag::Path, vec!["M", ",20 L30,40"]),
        ] {
            let t = Template::new(tag, parts(&text));
            assert!(t.plan.is_some());
            for value in [
                V::Num(10.0),
                V::Str("10,15 30".into()),
                V::Str("".into()),
                V::Str("bad".into()),
                V::Bool(false),
            ] {
                let values = [value];
                assert_eq!(t.resolve(&values), parsed(tag, &t.parts, &values));
            }
        }
    }
}
