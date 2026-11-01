//! The cases made from the support table, the style properties the engine takes
//! (`valle_engine::style::properties`):
//!
//! - `css/properties/<property>.tsx`: a sample of the property for each value [`ROWS`] gives it,
//!   compared with Chrome as any css case is;
//! - `errors/values/<property>.<n>.tsx`: the sample with a value the engine must refuse. Its case
//!   asks only for an error: the css case shows the engine takes the sample, so the error is the
//!   value's;
//! - `errors/properties/<property>.tsx`: each other property the pinned Chrome knows, which the
//!   engine must refuse.

use std::collections::{BTreeMap, BTreeSet};

/// Samples of properties: `PROPERTY: VALUE` stands for the declaration, and values are written
/// as JS.
struct Row {
    properties: &'static [&'static str],
    sample: &'static str,
    /// Values the engine takes, which it lays out and paints as Chrome does.
    values: &'static [&'static str],
    refused: &'static [&'static str],
}

const COMPOSITION: &str =
    "export const composition = { width: 256, height: 256, fps: 1, duration: 1 };";

const SIDES: &[&str] = &[
    "top",
    "right",
    "bottom",
    "left",
    "insetInlineStart",
    "insetInlineEnd",
    "insetBlockStart",
    "insetBlockEnd",
];
const MARGINS: &[&str] = &[
    "marginTop",
    "marginRight",
    "marginBottom",
    "marginLeft",
    "marginInlineStart",
    "marginInlineEnd",
    "marginBlockStart",
    "marginBlockEnd",
];
const PADDINGS: &[&str] = &[
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "paddingInlineStart",
    "paddingInlineEnd",
    "paddingBlockStart",
    "paddingBlockEnd",
];

const CONTENT_ALIGN: &[&str] = &[
    "\"normal\"",
    "\"stretch\"",
    "\"center\"",
    "\"start\"",
    "\"end\"",
    "\"flex-start\"",
    "\"flex-end\"",
    "\"space-between\"",
    "\"space-around\"",
    "\"space-evenly\"",
];
const SELF_ALIGN: &[&str] = &[
    "\"normal\"",
    "\"stretch\"",
    "\"center\"",
    "\"start\"",
    "\"end\"",
    "\"flex-start\"",
    "\"flex-end\"",
    "\"self-start\"",
    "\"self-end\"",
    "\"baseline\"",
];

const ROWS: &[Row] = &[
    Row {
        properties: &["display"],
        sample: r##"<View style={{ display: "flex", background: "#e8eaf6" }}><View style={{ padding: 2, background: "#7986cb", PROPERTY: VALUE }}><View style={{ width: 20, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 6, background: "#3949ab" }} /></View></View>"##,
        values: &[
            "\"block\"",
            "\"flex\"",
            "\"none\"",
            "\"inline-block\"",
            "\"inline-flex\"",
            "\"inline\"",
            "\" Flex \"",
        ],
        refused: &[
            "\"grid\"",
            "\"inline-grid\"",
            "\"table\"",
            "\"table-cell\"",
            "\"list-item\"",
            "\"contents\"",
            "\"flow-root\"",
            "1",
        ],
    },
    Row {
        properties: &["position"],
        sample: r##"<View style={{ position: "relative", height: 16, background: "#e8eaf6" }}><View style={{ width: 30, height: 8, top: 4, left: 10, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "\"static\"",
            "\"relative\"",
            "\"absolute\"",
            "\" Absolute \"",
        ],
        refused: &["\"fixed\"", "\"sticky\"", "\"float\"", "0"],
    },
    Row {
        properties: SIDES,
        sample: r##"<View style={{ position: "relative", height: 16, background: "#e8eaf6" }}><View style={{ position: "absolute", width: 30, height: 8, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "5",
            "2.5",
            "-3",
            "\"25%\"",
            "\"-10%\"",
            "\"0.5em\"",
            "\"1rem\"",
            "\"auto\"",
        ],
        refused: &["\"1vh\"", "\"calc(5px)\"", "\"5\"", "true", "NaN"],
    },
    Row {
        properties: &["inset"],
        sample: r##"<View style={{ position: "relative", height: 16, background: "#e8eaf6" }}><View style={{ position: "absolute", width: 30, height: 8, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "4",
            "\"2px 8px\"",
            "\"1px 2px 3px\"",
            "\"1px 2px 3px 4px\"",
            "\"auto\"",
            "\"10% auto\"",
            "\"auto 0 auto auto\"",
        ],
        refused: &["\"1px 2px 3px 4px 5px\"", "\"\"", "\"1px, 2px\"", "true"],
    },
    Row {
        properties: &["insetInline", "insetBlock"],
        sample: r##"<View style={{ position: "relative", height: 16, background: "#e8eaf6" }}><View style={{ position: "absolute", width: 30, height: 8, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "4",
            "-3",
            "\"2px 8px\"",
            "\"auto\"",
            "\"10% auto\"",
            "\"auto 0.5em\"",
        ],
        refused: &["\"1px 2px 3px\"", "\"\"", "true"],
    },
    Row {
        properties: &["width"],
        sample: r##"<View style={{ height: 6, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &[
            "0",
            "37",
            "12.3",
            "\"50%\"",
            "\"33.3%\"",
            "\"2.5em\"",
            "\"1.5rem\"",
            "\"auto\"",
            "\" 40PX \"",
            "300",
        ],
        refused: &[
            "-1",
            "\"-1px\"",
            "\"10\"",
            "\"1vw\"",
            "\"calc(10px)\"",
            "\"fit-content\"",
            "\"none\"",
            "true",
        ],
    },
    Row {
        properties: &["minWidth"],
        sample: r##"<View style={{ width: 20, height: 6, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &["0", "40", "10", "\"50%\"", "\"3em\"", "\"auto\""],
        refused: &["-1", "\"none\"", "\"min-content\""],
    },
    Row {
        properties: &["maxWidth"],
        sample: r##"<View style={{ height: 6, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &["0", "40", "\"50%\"", "\"3em\"", "\"none\"", "400"],
        refused: &["-1", "\"auto\"", "\"max-content\""],
    },
    Row {
        properties: &["height"],
        sample: r##"<View style={{ height: 20, background: "#e8eaf6" }}><View style={{ width: 40, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "7",
            "3.5",
            "\"50%\"",
            "\"1.5em\"",
            "\"0.5rem\"",
            "\"auto\"",
            "20",
        ],
        refused: &["-1", "\"1vh\"", "\"fit-content\"", "true"],
    },
    Row {
        properties: &["minHeight"],
        sample: r##"<View style={{ height: 20, background: "#e8eaf6" }}><View style={{ width: 40, height: 4, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &["0", "10", "2", "\"50%\"", "\"1em\"", "\"auto\""],
        refused: &["-1", "\"none\""],
    },
    Row {
        properties: &["maxHeight"],
        sample: r##"<View style={{ height: 20, background: "#e8eaf6" }}><View style={{ width: 40, height: 16, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &["0", "10", "\"50%\"", "\"0.5em\"", "\"none\"", "40"],
        refused: &["-1", "\"auto\""],
    },
    Row {
        properties: &["boxSizing"],
        sample: r##"<View style={{ width: 60, height: 10, padding: 4, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &["\"border-box\"", "\"content-box\"", "\" CONTENT-BOX \""],
        refused: &["\"padding-box\"", "\"margin-box\"", "1"],
    },
    Row {
        properties: &["aspectRatio"],
        sample: r##"<View style={{ width: 30, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &[
            "1",
            "2",
            "0.5",
            "\"16 / 9\"",
            "\"3/2\"",
            "\"auto\"",
            "\"auto 2\"",
            "\"0\"",
        ],
        refused: &["-1", "\"16:9\"", "\"1 / -2\"", "\"1 2\"", "true"],
    },
    Row {
        properties: MARGINS,
        sample: r##"<View style={{ padding: 1, background: "#e8eaf6" }}><View style={{ width: 100, height: 6, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "5",
            "-3",
            "2.5",
            "\"10%\"",
            "\"0.5em\"",
            "\"1rem\"",
            "\"auto\"",
        ],
        refused: &["\"1vw\"", "\"5\"", "\"calc(1px)\"", "true"],
    },
    Row {
        properties: &["margin"],
        sample: r##"<View style={{ padding: 1, background: "#e8eaf6" }}><View style={{ width: 100, height: 6, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "4",
            "\"2px 8px\"",
            "\"1px 2px 3px\"",
            "\"1px 2px 3px 4px\"",
            "\"auto\"",
            "\"0 auto\"",
            "\"-2px 5%\"",
        ],
        refused: &["\"1px 2px 3px 4px 5px\"", "\"\"", "\"1px, 2px\""],
    },
    Row {
        properties: &["marginInline", "marginBlock"],
        sample: r##"<View style={{ padding: 1, background: "#e8eaf6" }}><View style={{ width: 100, height: 6, background: "#3949ab", PROPERTY: VALUE }} /></View>"##,
        values: &[
            "0",
            "4",
            "-3",
            "\"2px 8px\"",
            "\"auto\"",
            "\"0 auto\"",
            "\"-2px 5%\"",
        ],
        refused: &["\"1px 2px 3px\"", "\"\"", "\"1px, 2px\""],
    },
    Row {
        properties: PADDINGS,
        sample: r##"<View style={{ width: 100, background: "#7986cb", PROPERTY: VALUE }}><View style={{ height: 4, background: "#3949ab" }} /></View>"##,
        values: &[
            "0",
            "5",
            "2.5",
            "\"10%\"",
            "\"0.5em\"",
            "\"1rem\"",
            "\" 3PX \"",
        ],
        refused: &["-1", "\"-1px\"", "\"auto\"", "\"1vw\"", "true"],
    },
    Row {
        properties: &["padding"],
        sample: r##"<View style={{ width: 100, background: "#7986cb", PROPERTY: VALUE }}><View style={{ height: 4, background: "#3949ab" }} /></View>"##,
        values: &[
            "0",
            "4",
            "\"2px 8px\"",
            "\"1px 2px 3px\"",
            "\"1px 2px 3px 4px\"",
            "\"5%\"",
        ],
        refused: &["\"auto\"", "\"1px 2px 3px 4px 5px\"", "-2"],
    },
    Row {
        properties: &["paddingInline", "paddingBlock"],
        sample: r##"<View style={{ width: 100, background: "#7986cb", PROPERTY: VALUE }}><View style={{ height: 4, background: "#3949ab" }} /></View>"##,
        values: &["0", "4", "\"2px 8px\"", "\"5%\"", "\"0.5em 1px\""],
        refused: &["\"auto\"", "\"1px 2px 3px\"", "-2"],
    },
    Row {
        properties: &["flexDirection"],
        sample: r##"<View style={{ display: "flex", height: 30, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 20, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 8, background: "#7986cb" }} /></View>"##,
        values: &[
            "\"row\"",
            "\"column\"",
            "\"row-reverse\"",
            "\"column-reverse\"",
        ],
        refused: &["\"vertical\"", "\"row column\"", "1"],
    },
    Row {
        properties: &["flexWrap"],
        sample: r##"<View style={{ display: "flex", width: 80, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 30, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 8, background: "#7986cb" }} /><View style={{ width: 30, height: 4, background: "#3949ab" }} /></View>"##,
        values: &["\"nowrap\"", "\"wrap\"", "\"wrap-reverse\""],
        refused: &["\"yes\"", "true"],
    },
    Row {
        properties: &["flexGrow"],
        sample: r##"<View style={{ display: "flex", height: 8, background: "#e8eaf6" }}><View style={{ width: 20, background: "#3949ab", PROPERTY: VALUE }} /><View style={{ width: 20, flexGrow: 1, background: "#7986cb" }} /></View>"##,
        values: &["0", "1", "2.5", "0.25", "\"3\""],
        refused: &["-1", "\"1px\"", "\"auto\"", "true"],
    },
    Row {
        properties: &["flexShrink"],
        sample: r##"<View style={{ display: "flex", width: 100, height: 8, background: "#e8eaf6" }}><View style={{ width: 80, background: "#3949ab", PROPERTY: VALUE }} /><View style={{ width: 80, background: "#7986cb" }} /></View>"##,
        values: &["0", "1", "3", "0.5", "\"2\""],
        refused: &["-1", "\"1px\""],
    },
    Row {
        properties: &["flexBasis"],
        sample: r##"<View style={{ display: "flex", width: 200, height: 8, background: "#e8eaf6" }}><View style={{ background: "#3949ab", PROPERTY: VALUE }} /><View style={{ flexGrow: 1, background: "#7986cb" }} /></View>"##,
        values: &["0", "40", "\"30%\"", "\"2em\"", "\"auto\"", "250"],
        refused: &["-5", "\"content\"", "\"1vw\""],
    },
    Row {
        properties: &["flex"],
        sample: r##"<View style={{ display: "flex", width: 200, height: 8, background: "#e8eaf6" }}><View style={{ background: "#3949ab", PROPERTY: VALUE }} /><View style={{ width: 30, background: "#7986cb" }} /></View>"##,
        values: &[
            "1",
            "0",
            "2",
            "\"none\"",
            "\"auto\"",
            "\"1 1 50px\"",
            "\"2 3\"",
            "\"40px\"",
            "\"1 30%\"",
            "\"0 0 auto\"",
        ],
        refused: &["\"1 2 3\"", "\"1px 2px\"", "-1", "\"initial\""],
    },
    Row {
        properties: &["order"],
        sample: r##"<View style={{ display: "flex", height: 10, background: "#e8eaf6" }}><View style={{ width: 20, background: "#3949ab" }} /><View style={{ width: 20, marginLeft: -10, background: "#e53935", PROPERTY: VALUE }} /><View style={{ width: 20, marginLeft: -10, background: "#7986cb" }} /></View>"##,
        values: &["0", "1", "-1", "2", "\"3\""],
        refused: &["1.5", "\"1px\"", "\"auto\"", "true"],
    },
    Row {
        properties: &["justifyContent"],
        sample: r##"<View style={{ display: "flex", height: 8, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 20, background: "#3949ab" }} /><View style={{ width: 30, background: "#7986cb" }} /><View style={{ width: 10, background: "#3949ab" }} /></View>"##,
        values: CONTENT_ALIGN,
        refused: &["\"left\"", "\"baseline\"", "\"safe center\"", "1"],
    },
    Row {
        properties: &["alignItems"],
        sample: r##"<View style={{ display: "flex", height: 16, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 20, height: 6, background: "#3949ab" }} /><View style={{ width: 20, background: "#7986cb" }} /><Text style={{ fontSize: 10 }}>Ag</Text></View>"##,
        values: SELF_ALIGN,
        refused: &[
            "\"left\"",
            "\"auto\"",
            "\"first baseline\"",
            "\"space-between\"",
        ],
    },
    Row {
        properties: &["alignSelf"],
        sample: r##"<View style={{ display: "flex", height: 16, background: "#e8eaf6" }}><View style={{ width: 20, height: 6, background: "#3949ab" }} /><View style={{ width: 20, background: "#7986cb", PROPERTY: VALUE }} /><Text style={{ fontSize: 10 }}>Ag</Text></View>"##,
        values: &[
            "\"auto\"",
            "\"normal\"",
            "\"stretch\"",
            "\"center\"",
            "\"start\"",
            "\"end\"",
            "\"flex-start\"",
            "\"flex-end\"",
            "\"self-start\"",
            "\"self-end\"",
            "\"baseline\"",
        ],
        refused: &["\"left\"", "\"space-between\"", "1"],
    },
    Row {
        properties: &["alignContent"],
        sample: r##"<View style={{ display: "flex", flexWrap: "wrap", width: 100, height: 20, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 40, height: 4, background: "#3949ab" }} /><View style={{ width: 40, height: 6, background: "#7986cb" }} /><View style={{ width: 40, height: 4, background: "#3949ab" }} /></View>"##,
        values: CONTENT_ALIGN,
        refused: &["\"left\"", "\"baseline\"", "1"],
    },
    Row {
        properties: &["gap"],
        sample: r##"<View style={{ display: "flex", flexWrap: "wrap", width: 100, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 30, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 6, background: "#7986cb" }} /><View style={{ width: 30, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 6, background: "#7986cb" }} /></View>"##,
        values: &[
            "0",
            "4",
            "\"2px 10px\"",
            "\"normal\"",
            "\"10%\"",
            "\"0.5em\"",
        ],
        refused: &["-1", "\"1px 2px 3px\"", "\"auto\"", "true"],
    },
    Row {
        properties: &["rowGap", "columnGap"],
        sample: r##"<View style={{ display: "flex", flexWrap: "wrap", width: 100, background: "#e8eaf6", PROPERTY: VALUE }}><View style={{ width: 30, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 6, background: "#7986cb" }} /><View style={{ width: 30, height: 6, background: "#3949ab" }} /><View style={{ width: 30, height: 6, background: "#7986cb" }} /></View>"##,
        values: &["0", "4", "\"normal\"", "\"10%\"", "\"0.5em\""],
        refused: &["-1", "\"1px 2px\""],
    },
    Row {
        properties: &["background"],
        sample: r##"<View style={{ height: 8, PROPERTY: VALUE }} />"##,
        values: &[
            "\"#3949ab\"",
            "\"#36c\"",
            "\"#3949ab80\"",
            "\"#36c8\"",
            "\"red\"",
            "\"RebeccaPurple\"",
            "\"rgb(10 20 30)\"",
            "\"rgba(57, 73, 171, 0.5)\"",
            "\"rgb(20% 40% 60% / 50%)\"",
            "\"hsl(200 50% 40%)\"",
            "\"hsla(120, 60%, 50%, 0.3)\"",
            "\"transparent\"",
            "\"none\"",
        ],
        refused: &[
            "\"url(a.png)\"",
            "\"linear-gradient(red, blue)\"",
            "\"red blue\"",
            "\"#12345\"",
            "1",
        ],
    },
    Row {
        properties: &["backgroundColor"],
        sample: r##"<View style={{ height: 8, PROPERTY: VALUE }} />"##,
        values: &[
            "\"#3949ab\"",
            "\"#36c8\"",
            "\"red\"",
            "\" RED \"",
            "\"rgb(10 20 30)\"",
            "\"rgba(57, 73, 171, 0.5)\"",
            "\"hsl(200 50% 40%)\"",
            "\"transparent\"",
        ],
        refused: &["\"none\"", "\"blue red\"", "\"#ggg\"", "0"],
    },
    Row {
        properties: &["borderRadius"],
        sample: r##"<View style={{ width: 60, height: 16, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &["0", "4", "8", "20", "2.5", "\"0.5em\"", "\"1rem\""],
        refused: &["\"50%\"", "\"4px 8px\"", "-1", "\"4px / 2px\""],
    },
    Row {
        properties: &["opacity"],
        sample: r##"<View style={{ height: 8, background: "#3949ab", PROPERTY: VALUE }} />"##,
        values: &[
            "0", "1", "0.5", "0.25", "\"50%\"", "\"0.75\"", "2", "-1", "\"150%\"",
        ],
        refused: &["true", "\"half\"", "\"1px\""],
    },
    Row {
        properties: &["color"],
        sample: r##"<Text style={{ fontSize: 10, PROPERTY: VALUE }}>Color</Text>"##,
        values: &[
            "\"#3949ab\"",
            "\"red\"",
            "\"rgb(10 20 30 / 50%)\"",
            "\"transparent\"",
            "\"hsl(0 0% 50%)\"",
        ],
        refused: &["\"none\"", "1", "\"url(a.png)\""],
    },
    Row {
        properties: &["fontSize"],
        sample: r##"<View style={{ display: "flex" }}><Text style={{ PROPERTY: VALUE }}>Size</Text></View>"##,
        values: &[
            "0",
            "9",
            "12.5",
            "\"20px\"",
            "\"1.5em\"",
            "\"0.75rem\"",
            "\"80%\"",
            "\" 14PX \"",
        ],
        refused: &["-1", "\"large\"", "\"1vw\"", "\"smaller\"", "true"],
    },
    // The font's masters, and the keywords. Between masters Chrome keeps the variation deltas of
    // glyph advances unrounded where HarfBuzz rounds them to font units, and text comes out a few
    // 1/64 px off: this sample is two units off at 300 and 550, one at 700.
    Row {
        properties: &["fontWeight"],
        sample: r##"<View style={{ display: "flex" }}><Text style={{ fontSize: 10, PROPERTY: VALUE }}>Weight of the text</Text></View>"##,
        values: &[
            "100",
            "400",
            "900",
            "\"900\"",
            "\"normal\"",
            "\"bold\"",
            "\"bolder\"",
            "\"lighter\"",
            "1",
            "1000",
        ],
        refused: &["0", "1001", "\"heavy\"", "true", "\"100px\""],
    },
    Row {
        properties: &["fontFamily"],
        sample: r##"<View style={{ display: "flex" }}><Text style={{ fontSize: 10, PROPERTY: VALUE }}>Family</Text></View>"##,
        values: &["\"sans-serif\"", "\"SANS-SERIF\"", "\" sans-serif \""],
        refused: &[
            "\"serif\"",
            "\"monospace\"",
            "\"cursive\"",
            "\"Inter, sans-serif\"",
            "\"'Inter'\"",
            "\"\"",
            "1",
        ],
    },
    Row {
        properties: &["lineHeight"],
        sample: r##"<View style={{ display: "flex" }}><Text style={{ fontSize: 10, PROPERTY: VALUE }}>Line height</Text></View>"##,
        values: &[
            "\"normal\"",
            "1",
            "1.425",
            "0",
            "0.5",
            "\"1.2\"",
            "\"16.9px\"",
            "\"150%\"",
            "\"2em\"",
            "\"0.8rem\"",
        ],
        refused: &["-1", "\"-1px\"", "\"1vw\"", "true", "\"normal 2\""],
    },
    Row {
        properties: &["textAlign"],
        sample: r##"<View style={{ width: 120, background: "#e8eaf6" }}><Text style={{ fontSize: 10, PROPERTY: VALUE }}>Aligned text that wraps onto two lines</Text></View>"##,
        values: &[
            "\"start\"",
            "\"end\"",
            "\"left\"",
            "\"right\"",
            "\"center\"",
            "\" CENTER \"",
        ],
        refused: &["\"justify\"", "\"middle\"", "\"match-parent\"", "1"],
    },
    Row {
        properties: &["letterSpacing"],
        sample: r##"<View style={{ display: "flex" }}><Text style={{ fontSize: 10, PROPERTY: VALUE }}>Spaced</Text></View>"##,
        values: &[
            "0",
            "1",
            "-0.5",
            "1.5",
            "\"0.2em\"",
            "\"0.1rem\"",
            "\"normal\"",
        ],
        refused: &["\"10%\"", "\"1vw\"", "true", "\"wide\""],
    },
];

/// The cases, by path from `tests/cases`, and their source; `chrome` is every CSS property the
/// pinned Chrome knows, by its CSS name.
pub fn cases(chrome: &[String]) -> Result<BTreeMap<String, String>, String> {
    let supported: BTreeSet<&str> = valle_engine::style::properties().collect();
    let known: BTreeSet<String> = chrome.iter().map(|name| key(name)).collect();
    let mut rows = BTreeMap::new();
    for row in ROWS {
        if !row.sample.contains("PROPERTY: VALUE") {
            return Err(format!(
                "the sample for {:?} has no PROPERTY: VALUE",
                row.properties
            ));
        }
        for &p in row.properties {
            if !supported.contains(p) {
                return Err(format!("{p} has a row but the engine does not take it"));
            }
            if rows.insert(p, row).is_some() {
                return Err(format!("{p} has two rows"));
            }
        }
    }
    let mut out = BTreeMap::new();
    for &p in &supported {
        let row = rows
            .get(p)
            .ok_or_else(|| format!("{p} has no row in properties.rs"))?;
        if !known.contains(p) {
            return Err(format!("the engine takes {p}, which Chrome does not know"));
        }
        let sample = |v: &str| row.sample.replace("PROPERTY", p).replace("VALUE", v);
        let mut css = format!(
            "{COMPOSITION}\n\n// {p}: each value the engine takes, from the support table.\n\
             export default function Scene() {{\n  return (\n    <View style={{{{ padding: 4 }}}}>\n"
        );
        for &v in row.values {
            css += &format!(
                "      <View style={{{{ paddingBottom: 4 }}}}>{}</View>\n",
                sample(v)
            );
        }
        css += "    </View>\n  );\n}\n";
        out.insert(format!("css/properties/{p}.tsx"), css);
        for (n, &v) in row.refused.iter().enumerate() {
            let case = format!("errors/values/{p}.{n}.tsx");
            let source = format!(
                "// error: {case}:…\n{COMPOSITION}\n\n// {p}: {v}, which the engine refuses.\n\
                 export default function Scene() {{\n  return (\n    <View style={{{{ padding: 4 }}}}>\n\
                 \x20     <View style={{{{ paddingBottom: 4 }}}}>{}</View>\n    </View>\n  );\n}}\n",
                sample(v)
            );
            out.insert(case, source);
        }
    }
    for k in known.iter().filter(|k| !supported.contains(k.as_str())) {
        let case = format!("errors/properties/{k}.tsx");
        let source = format!(
            "// error: {case}:5:10: style {k} is not supported…\n{COMPOSITION}\n\n\
             export default function Scene() {{\n  return <View style={{{{ {k}: \"initial\" }}}} />;\n}}\n"
        );
        out.insert(case, source);
    }
    Ok(out)
}

/// A CSS property's name as a React style key: `background-color` is `backgroundColor`, and
/// `-webkit-text-stroke` is `WebkitTextStroke`.
fn key(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, word) in name.split('-').enumerate() {
        let mut chars = word.chars();
        match chars.next() {
            Some(c) if i > 0 => out.extend(c.to_uppercase().chain(chars)),
            Some(c) => out.extend(std::iter::once(c).chain(chars)),
            None => {}
        }
    }
    out
}
