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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_unicode_bidi_value_opens_and_closes_its_controls() {
        // The controls each value is equivalent to, ltr then rtl, as CSS Writing Modes 4 maps
        // them; Chromium lays mixed Latin, Hebrew and digit text out the same with either.
        let cases = [
            (Bidi::Normal, ["", ""], ["", ""]),
            (
                Bidi::Embed,
                ["\u{202a}", "\u{202c}"],
                ["\u{202b}", "\u{202c}"],
            ),
            (
                Bidi::Override,
                ["\u{202d}", "\u{202c}"],
                ["\u{202e}", "\u{202c}"],
            ),
            (
                Bidi::Isolate,
                ["\u{2066}", "\u{2069}"],
                ["\u{2067}", "\u{2069}"],
            ),
            (
                Bidi::IsolateOverride,
                ["\u{2068}\u{202d}", "\u{202c}\u{2069}"],
                ["\u{2068}\u{202e}", "\u{202c}\u{2069}"],
            ),
            (
                Bidi::Plaintext,
                ["\u{2068}", "\u{2069}"],
                ["\u{2068}", "\u{2069}"],
            ),
        ];
        for (bidi, ltr, rtl) in cases {
            for (rtl_, want) in [(false, ltr), (true, rtl)] {
                let scope = Scope {
                    node: 0,
                    bidi,
                    rtl: rtl_,
                };
                let (open, close) = scope.controls();
                assert_eq!([open, close], want, "{bidi:?}, rtl {rtl_}");
            }
        }
    }
}
