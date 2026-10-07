use serde::{Serialize, Serializer};
use valle_motion::canonical_bytes;

#[derive(Serialize)]
struct Unit;
#[derive(Serialize)]
struct Newtype(u8);
#[derive(Serialize)]
struct Tuple(i16, char);
#[derive(Serialize)]
enum Variant {
    Unit,
    Newtype(u16),
    Tuple(u8, i8),
    Struct { first: i32, second: u64 },
}
struct Bytes;
impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&[1, 2, 255])
    }
}
struct Refused;
impl Serialize for Refused {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("refused payload"))
    }
}

#[test]
fn canonical_serialization_handles_serde_shapes_and_rejects_lossy_values() {
    assert_eq!(canonical_bytes(&Unit).unwrap(), b"null");
    assert_eq!(canonical_bytes(&Newtype(7)).unwrap(), b"7");
    assert_eq!(
        canonical_bytes(&Tuple(-2, '字')).unwrap(),
        "[-2,\"字\"]".as_bytes()
    );
    assert_eq!(canonical_bytes(&Variant::Unit).unwrap(), b"\"Unit\"");
    assert_eq!(
        canonical_bytes(&Variant::Newtype(3)).unwrap(),
        b"{\"Newtype\":3}"
    );
    assert_eq!(
        canonical_bytes(&Variant::Tuple(2, -1)).unwrap(),
        b"{\"Tuple\":[2,-1]}"
    );
    assert_eq!(
        canonical_bytes(&Variant::Struct {
            first: 1,
            second: 2
        })
        .unwrap(),
        b"{\"Struct\":{\"first\":1,\"second\":2}}"
    );
    assert_eq!(canonical_bytes(&Bytes).unwrap(), b"[1,2,255]");
    canonical_bytes(&(1_i128, 2_u128, 3_i32, 4_u16, 5_u8, 6_i8, 7_i16, ())).unwrap();
    let error = canonical_bytes(&Refused).unwrap_err();
    assert!(error.to_string().contains("refused payload"));
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(canonical_bytes(&[Some(value)]).is_err());
    }
    assert!(canonical_bytes(&f32::NAN).is_err());
}

#[test]
fn layout_and_formula_registries_offer_explicit_discovery() {
    assert!(valle_motion::layout::supports_css_property("font-size"));
    assert!(!valle_motion::layout::supports_css_property("made-up-css"));
    let empty = valle_motion::math_formula::FormulaFontRegistry::new();
    assert!(empty.is_empty());
    let registry = valle_motion::math_formula::FormulaFontRegistry::load_default().unwrap();
    assert!(!registry.is_empty());
    let face = valle_motion::math_formula::formula_faces()[0];
    let font = registry.font_face(face.ratex_name, 18.0).unwrap();
    assert_eq!(font.size, 18.0);
    assert_eq!(font.weight, 400);
    assert!(registry.font_face("not-a-formula-font", 18.0).is_err());
    let raw = registry.raw_cmap_gid("Main-Regular", 'A').unwrap().unwrap();
    assert_eq!(
        raw,
        registry.glyph_id("Main-Regular", u32::from('A')).unwrap()
    );
    assert_eq!(
        registry.raw_cmap_gid("Main-Regular", '\u{1f600}').unwrap(),
        None
    );
    assert!(registry.raw_cmap_gid("not-a-formula-font", 'A').is_err());
}
