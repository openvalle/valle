//! CSS bidi scopes survive inheritance without making unicode-bidi itself inherited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bidi {
    #[default]
    Normal,
    Embed,
    Override,
    Isolate,
    IsolateOverride,
    Plaintext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scope {
    pub node: usize,
    pub bidi: Bidi,
    pub rtl: bool,
}

impl Scope {
    pub fn controls(self) -> (&'static str, &'static str) {
        match (self.bidi, self.rtl) {
            (Bidi::Normal, _) => ("", ""),
            (Bidi::Embed, false) => ("\u{202a}", "\u{202c}"),
            (Bidi::Embed, true) => ("\u{202b}", "\u{202c}"),
            (Bidi::Override, false) => ("\u{202d}", "\u{202c}"),
            (Bidi::Override, true) => ("\u{202e}", "\u{202c}"),
            (Bidi::Isolate, false) => ("\u{2066}", "\u{2069}"),
            (Bidi::Isolate, true) => ("\u{2067}", "\u{2069}"),
            (Bidi::IsolateOverride, false) => ("\u{2068}\u{202d}", "\u{202c}\u{2069}"),
            (Bidi::IsolateOverride, true) => ("\u{2068}\u{202e}", "\u{202c}\u{2069}"),
            (Bidi::Plaintext, _) => ("\u{2068}", "\u{2069}"),
        }
    }
}
