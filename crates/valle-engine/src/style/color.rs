//! CSS colors. A color value is sRGB as CSS writes it, encoded, with straight alpha: browsers
//! composite these values, and so does the working space.

use super::css::{self, Num};

/// Encoded sRGB and straight alpha, each in [0, 1].
pub type Rgba = [f64; 4];

pub const TRANSPARENT: Rgba = [0.0; 4];
pub const BLACK: Rgba = [0.0, 0.0, 0.0, 1.0];

/// A color as CSS writes it, lowercased.
pub fn parse(s: &str) -> Result<Rgba, String> {
    let bad = || format!("{s} is not a color");
    if let Some(h) = s.strip_prefix('#') {
        return hex(h).ok_or_else(bad);
    }
    if s == "transparent" {
        return Ok(TRANSPARENT);
    }
    if let Ok(i) = NAMED.binary_search_by(|(n, _)| (*n).cmp(s)) {
        return Ok(encoded(NAMED[i].1.map(|v| f64::from(v) / 255.0), 1.0));
    }
    let (name, args) = css::function(s).ok_or_else(bad)?;
    let c = match name {
        "rgb" | "rgba" => rgb(args),
        "hsl" | "hsla" => hsl(args),
        _ => None,
    };
    c.ok_or_else(bad)
}

fn encoded(rgb: [f64; 3], alpha: f64) -> Rgba {
    let [r, g, b] = rgb.map(|c| c.clamp(0.0, 1.0));
    [r, g, b, alpha.clamp(0.0, 1.0)]
}

fn hex(h: &str) -> Option<Rgba> {
    if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&h[i..=i], 16).map(|v| v * 17);
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16);
    let c = match h.len() {
        3 => [digit(0), digit(1), digit(2), Ok(255)],
        4 => [digit(0), digit(1), digit(2), digit(3)],
        6 => [byte(0), byte(2), byte(4), Ok(255)],
        8 => [byte(0), byte(2), byte(4), byte(6)],
        _ => return None,
    };
    let [r, g, b, a] = c.map(|v| v.map(|v| f64::from(v) / 255.0));
    Some(encoded([r.ok()?, g.ok()?, b.ok()?], a.ok()?))
}

/// A color function's channels and alpha, in modern (`a b c / d`) or legacy (`a, b, c, d`)
/// syntax, and whether it was legacy.
fn channels(args: &str) -> Option<([Num<'_>; 3], Option<Num<'_>>, bool)> {
    let legacy = args.contains(',');
    let (main, alpha) = if legacy {
        let parts = css::commas(args);
        match parts.len() {
            3 => (parts, None),
            4 => (parts[..3].to_vec(), Some(parts[3])),
            _ => return None,
        }
    } else {
        let (main, alpha) = match args.split_once('/') {
            Some((m, a)) => (m, Some(a.trim())),
            None => (args, None),
        };
        (css::words(main), alpha)
    };
    let [a, b, c] = main[..] else { return None };
    let nums = [css::num(a)?, css::num(b)?, css::num(c)?];
    let alpha = match alpha {
        Some(a) => Some(css::num(a).filter(|n| n.unit.is_empty() || n.unit == "%")?),
        None => None,
    };
    Some((nums, alpha, legacy))
}

fn alpha(a: Option<Num>) -> f64 {
    match a {
        None => 1.0,
        Some(n) if n.unit == "%" => n.value / 100.0,
        Some(n) => n.value,
    }
}

fn rgb(args: &str) -> Option<Rgba> {
    let (c, a, legacy) = channels(args)?;
    if c.iter().any(|n| !n.unit.is_empty() && n.unit != "%") {
        return None;
    }
    if legacy && c.iter().any(|n| (n.unit == "%") != (c[0].unit == "%")) {
        return None;
    }
    let channel = |n: &Num| match n.unit {
        "%" => n.value / 100.0,
        _ => n.value / 255.0,
    };
    Some(encoded(
        [channel(&c[0]), channel(&c[1]), channel(&c[2])],
        alpha(a),
    ))
}

/// Degrees.
fn hue(n: &Num) -> Option<f64> {
    Some(match n.unit {
        "" | "deg" => n.value,
        "rad" => n.value.to_degrees(),
        "grad" => n.value * 0.9,
        "turn" => n.value * 360.0,
        _ => return None,
    })
}

fn hsl(args: &str) -> Option<Rgba> {
    let (c, a, legacy) = channels(args)?;
    // Saturation and lightness: percentages, or (modern syntax) numbers on the same scale.
    let part = |n: &Num| match n.unit {
        "%" => Some(n.value / 100.0),
        "" if !legacy => Some(n.value / 100.0),
        _ => None,
    };
    let (h, s, l) = (hue(&c[0])?, part(&c[1])?.clamp(0.0, 1.0), part(&c[2])?);
    let l = l.clamp(0.0, 1.0);
    let h = h.rem_euclid(360.0);
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    Some(encoded([f(0.0), f(8.0), f(4.0)], alpha(a)))
}

/// CSS Color 4's named colors, sorted by name.
const NAMED: [(&str, [u8; 3]); 148] = [
    ("aliceblue", [240, 248, 255]),
    ("antiquewhite", [250, 235, 215]),
    ("aqua", [0, 255, 255]),
    ("aquamarine", [127, 255, 212]),
    ("azure", [240, 255, 255]),
    ("beige", [245, 245, 220]),
    ("bisque", [255, 228, 196]),
    ("black", [0, 0, 0]),
    ("blanchedalmond", [255, 235, 205]),
    ("blue", [0, 0, 255]),
    ("blueviolet", [138, 43, 226]),
    ("brown", [165, 42, 42]),
    ("burlywood", [222, 184, 135]),
    ("cadetblue", [95, 158, 160]),
    ("chartreuse", [127, 255, 0]),
    ("chocolate", [210, 105, 30]),
    ("coral", [255, 127, 80]),
    ("cornflowerblue", [100, 149, 237]),
    ("cornsilk", [255, 248, 220]),
    ("crimson", [220, 20, 60]),
    ("cyan", [0, 255, 255]),
    ("darkblue", [0, 0, 139]),
    ("darkcyan", [0, 139, 139]),
    ("darkgoldenrod", [184, 134, 11]),
    ("darkgray", [169, 169, 169]),
    ("darkgreen", [0, 100, 0]),
    ("darkgrey", [169, 169, 169]),
    ("darkkhaki", [189, 183, 107]),
    ("darkmagenta", [139, 0, 139]),
    ("darkolivegreen", [85, 107, 47]),
    ("darkorange", [255, 140, 0]),
    ("darkorchid", [153, 50, 204]),
    ("darkred", [139, 0, 0]),
    ("darksalmon", [233, 150, 122]),
    ("darkseagreen", [143, 188, 143]),
    ("darkslateblue", [72, 61, 139]),
    ("darkslategray", [47, 79, 79]),
    ("darkslategrey", [47, 79, 79]),
    ("darkturquoise", [0, 206, 209]),
    ("darkviolet", [148, 0, 211]),
    ("deeppink", [255, 20, 147]),
    ("deepskyblue", [0, 191, 255]),
    ("dimgray", [105, 105, 105]),
    ("dimgrey", [105, 105, 105]),
    ("dodgerblue", [30, 144, 255]),
    ("firebrick", [178, 34, 34]),
    ("floralwhite", [255, 250, 240]),
    ("forestgreen", [34, 139, 34]),
    ("fuchsia", [255, 0, 255]),
    ("gainsboro", [220, 220, 220]),
    ("ghostwhite", [248, 248, 255]),
    ("gold", [255, 215, 0]),
    ("goldenrod", [218, 165, 32]),
    ("gray", [128, 128, 128]),
    ("green", [0, 128, 0]),
    ("greenyellow", [173, 255, 47]),
    ("grey", [128, 128, 128]),
    ("honeydew", [240, 255, 240]),
    ("hotpink", [255, 105, 180]),
    ("indianred", [205, 92, 92]),
    ("indigo", [75, 0, 130]),
    ("ivory", [255, 255, 240]),
    ("khaki", [240, 230, 140]),
    ("lavender", [230, 230, 250]),
    ("lavenderblush", [255, 240, 245]),
    ("lawngreen", [124, 252, 0]),
    ("lemonchiffon", [255, 250, 205]),
    ("lightblue", [173, 216, 230]),
    ("lightcoral", [240, 128, 128]),
    ("lightcyan", [224, 255, 255]),
    ("lightgoldenrodyellow", [250, 250, 210]),
    ("lightgray", [211, 211, 211]),
    ("lightgreen", [144, 238, 144]),
    ("lightgrey", [211, 211, 211]),
    ("lightpink", [255, 182, 193]),
    ("lightsalmon", [255, 160, 122]),
    ("lightseagreen", [32, 178, 170]),
    ("lightskyblue", [135, 206, 250]),
    ("lightslategray", [119, 136, 153]),
    ("lightslategrey", [119, 136, 153]),
    ("lightsteelblue", [176, 196, 222]),
    ("lightyellow", [255, 255, 224]),
    ("lime", [0, 255, 0]),
    ("limegreen", [50, 205, 50]),
    ("linen", [250, 240, 230]),
    ("magenta", [255, 0, 255]),
    ("maroon", [128, 0, 0]),
    ("mediumaquamarine", [102, 205, 170]),
    ("mediumblue", [0, 0, 205]),
    ("mediumorchid", [186, 85, 211]),
    ("mediumpurple", [147, 112, 219]),
    ("mediumseagreen", [60, 179, 113]),
    ("mediumslateblue", [123, 104, 238]),
    ("mediumspringgreen", [0, 250, 154]),
    ("mediumturquoise", [72, 209, 204]),
    ("mediumvioletred", [199, 21, 133]),
    ("midnightblue", [25, 25, 112]),
    ("mintcream", [245, 255, 250]),
    ("mistyrose", [255, 228, 225]),
    ("moccasin", [255, 228, 181]),
    ("navajowhite", [255, 222, 173]),
    ("navy", [0, 0, 128]),
    ("oldlace", [253, 245, 230]),
    ("olive", [128, 128, 0]),
    ("olivedrab", [107, 142, 35]),
    ("orange", [255, 165, 0]),
    ("orangered", [255, 69, 0]),
    ("orchid", [218, 112, 214]),
    ("palegoldenrod", [238, 232, 170]),
    ("palegreen", [152, 251, 152]),
    ("paleturquoise", [175, 238, 238]),
    ("palevioletred", [219, 112, 147]),
    ("papayawhip", [255, 239, 213]),
    ("peachpuff", [255, 218, 185]),
    ("peru", [205, 133, 63]),
    ("pink", [255, 192, 203]),
    ("plum", [221, 160, 221]),
    ("powderblue", [176, 224, 230]),
    ("purple", [128, 0, 128]),
    ("rebeccapurple", [102, 51, 153]),
    ("red", [255, 0, 0]),
    ("rosybrown", [188, 143, 143]),
    ("royalblue", [65, 105, 225]),
    ("saddlebrown", [139, 69, 19]),
    ("salmon", [250, 128, 114]),
    ("sandybrown", [244, 164, 96]),
    ("seagreen", [46, 139, 87]),
    ("seashell", [255, 245, 238]),
    ("sienna", [160, 82, 45]),
    ("silver", [192, 192, 192]),
    ("skyblue", [135, 206, 235]),
    ("slateblue", [106, 90, 205]),
    ("slategray", [112, 128, 144]),
    ("slategrey", [112, 128, 144]),
    ("snow", [255, 250, 250]),
    ("springgreen", [0, 255, 127]),
    ("steelblue", [70, 130, 180]),
    ("tan", [210, 180, 140]),
    ("teal", [0, 128, 128]),
    ("thistle", [216, 191, 216]),
    ("tomato", [255, 99, 71]),
    ("turquoise", [64, 224, 208]),
    ("violet", [238, 130, 238]),
    ("wheat", [245, 222, 179]),
    ("white", [255, 255, 255]),
    ("whitesmoke", [245, 245, 245]),
    ("yellow", [255, 255, 0]),
    ("yellowgreen", [154, 205, 50]),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The color as bytes, rounded.
    fn bytes(s: &str) -> [u32; 4] {
        parse(s).unwrap().map(|v| (v * 255.0).round() as u32)
    }

    #[test]
    fn hex_and_names() {
        assert_eq!(bytes("#f80"), [255, 136, 0, 255]);
        assert_eq!(bytes("#f808"), [255, 136, 0, 136]);
        assert_eq!(bytes("#12abef"), [18, 171, 239, 255]);
        assert_eq!(bytes("#12abef80"), [18, 171, 239, 128]);
        assert_eq!(bytes("rebeccapurple"), [102, 51, 153, 255]);
        assert_eq!(parse("transparent"), Ok(TRANSPARENT));
        for bad in ["#12", "#12345", "#ggg", "#+12", "blurple", "rgb", ""] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(NAMED.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn color_functions() {
        assert_eq!(bytes("rgb(255 128 0)"), [255, 128, 0, 255]);
        assert_eq!(bytes("rgb(255, 128, 0, 0.5)"), [255, 128, 0, 128]);
        assert_eq!(bytes("rgba(100%, 40%, 0%, 50%)"), [255, 102, 0, 128]);
        assert_eq!(bytes("rgb(100% 128 0 / 25%)"), [255, 128, 0, 64]);
        // Out-of-range channels clamp, as CSS clamps them.
        assert_eq!(bytes("rgb(300 -5 0 / 2)"), [255, 0, 0, 255]);
        assert_eq!(bytes("hsl(120 100% 50%)"), [0, 255, 0, 255]);
        assert_eq!(bytes("hsl(0.5turn, 100%, 20%)"), [0, 102, 102, 255]);
        assert_eq!(bytes("hsla(-120deg 100 50 / 0.5)"), [0, 0, 255, 128]);
        assert_eq!(bytes("hsl(0 0% 100%)"), [255, 255, 255, 255]);
        for bad in [
            "rgb(1 2)",
            "rgb(1, 2 3)",
            "rgb(100%, 2, 3)",
            "rgb(1px 2 3)",
            "rgb(1 2 3 / 1px)",
            "hsl(120, 100, 50)",
            "hsl(1px 2% 3%)",
            "lab(50 0 0)",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
