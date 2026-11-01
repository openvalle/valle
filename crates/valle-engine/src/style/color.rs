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
        "hwb" => hwb(args),
        "lab" => lab(args, false),
        "lch" => lab(args, true),
        "oklab" => oklab_color(args, false),
        "oklch" => oklab_color(args, true),
        "color" => color(args),
        _ => None,
    };
    c.ok_or_else(bad)
}

fn encoded(rgb: [f64; 3], alpha: f64) -> Rgba {
    let [r, g, b] = rgb.map(|c| c.clamp(0.0, 1.0));
    [r, g, b, alpha.clamp(0.0, 1.0)]
}

/// An encoded sRGB channel in linear light, the curve mirrored below 0.
pub fn linear(c: f64) -> f64 {
    let a = c.abs();
    let v = match a <= 0.04045 {
        true => a / 12.92,
        false => ((a + 0.055) / 1.055).powf(2.4),
    };
    v.copysign(c)
}

/// The encoded sRGB channel of one in linear light, the curve mirrored below 0.
fn gamma(c: f64) -> f64 {
    let a = c.abs();
    let v = match a <= 0.0031308 {
        true => a * 12.92,
        false => 1.055 * a.powf(1.0 / 2.4) - 0.055,
    };
    v.copysign(c)
}

fn times(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

/// Linear-light sRGB of CIE XYZ, D65 white.
const XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [3.2409699419045226, -1.537383177570094, -0.4986107602930034],
    [-0.9692436362808796, 1.8759675015077202, 0.04155505740717559],
    [
        0.05563007969699366,
        -0.20397695888897652,
        1.0569715142428786,
    ],
];

/// CIE XYZ from a D50 white to D65, Bradford's.
const D50_TO_D65: [[f64; 3]; 3] = [
    [0.955473421488075, -0.02309845494876471, 0.06325924320057072],
    [
        -0.0283697093338637,
        1.0099953980813041,
        0.021041441191917323,
    ],
    [
        0.012314014864481998,
        -0.020507649298898964,
        1.330365926242124,
    ],
];

/// Encoded sRGB of CIE XYZ, D65 white.
fn from_xyz(xyz: [f64; 3]) -> [f64; 3] {
    times(XYZ_TO_SRGB, xyz).map(gamma)
}

/// The Oklab lightness, a and b of encoded sRGB channels.
pub fn oklab(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb.map(linear);
    let [l, m, s] = [
        0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b,
        0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b,
        0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b,
    ]
    .map(f64::cbrt);
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

/// Encoded sRGB channels of an Oklab lightness, a and b.
fn from_oklab([l, a, b]: [f64; 3]) -> [f64; 3] {
    let [l, m, s] = [
        l + 0.3963377774 * a + 0.2158037573 * b,
        l - 0.1055613458 * a - 0.0638541728 * b,
        l - 0.0894841775 * a - 1.2914855480 * b,
    ]
    .map(|c| c * c * c);
    [
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ]
    .map(gamma)
}

/// Encoded sRGB channels of a CIE Lab lightness, a and b, D50 white.
fn from_lab([l, a, b]: [f64; 3]) -> [f64; 3] {
    const KAPPA: f64 = 24389.0 / 27.0;
    const EPSILON: f64 = 216.0 / 24389.0;
    let f1 = (l + 16.0) / 116.0;
    let (f0, f2) = (a / 500.0 + f1, f1 - b / 200.0);
    let cube = |f: f64| match f * f * f > EPSILON {
        true => f * f * f,
        false => (116.0 * f - 16.0) / KAPPA,
    };
    let y = match l > KAPPA * EPSILON {
        true => f1 * f1 * f1,
        false => l / KAPPA,
    };
    let white = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
    let xyz = [cube(f0) * white[0], y, cube(f2) * white[2]];
    from_xyz(times(D50_TO_D65, xyz))
}

/// A polar color's a and b of its chroma and hue, degrees.
fn polar(chroma: f64, hue: f64) -> [f64; 2] {
    let h = hue.to_radians();
    [chroma * h.cos(), chroma * h.sin()]
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
    // `none`, a missing channel, is 0 where a color is drawn.
    fn num(s: &str, legacy: bool) -> Option<Num<'_>> {
        match s {
            "none" if !legacy => Some(Num {
                value: 0.0,
                unit: "",
            }),
            _ => css::num(s),
        }
    }
    let nums = [num(a, legacy)?, num(b, legacy)?, num(c, legacy)?];
    let alpha = match alpha {
        Some(a) => Some(num(a, legacy).filter(|n| n.unit.is_empty() || n.unit == "%")?),
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
    Some(encoded(from_hsl(h, s, l.clamp(0.0, 1.0)), alpha(a)))
}

/// Encoded sRGB channels of a hue, degrees, a saturation and a lightness, each 0–1.
fn from_hsl(h: f64, s: f64, l: f64) -> [f64; 3] {
    let h = h.rem_euclid(360.0);
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

/// A channel as a number, or a percentage of `full`.
fn scaled(n: &Num, full: f64) -> Option<f64> {
    match n.unit {
        "" => Some(n.value),
        "%" => Some(n.value / 100.0 * full),
        _ => None,
    }
}

fn hwb(args: &str) -> Option<Rgba> {
    let (c, a, legacy) = channels(args)?;
    if legacy {
        return None;
    }
    // Whiteness and blackness: percentages, or numbers on the same scale.
    let part = |n: &Num| match n.unit {
        "%" | "" => Some((n.value / 100.0).max(0.0)),
        _ => None,
    };
    let (h, w, b) = (hue(&c[0])?, part(&c[1])?, part(&c[2])?);
    if w + b >= 1.0 {
        return Some(encoded([w / (w + b); 3], alpha(a)));
    }
    let rgb = from_hsl(h, 1.0, 0.5).map(|c| c * (1.0 - w - b) + w);
    Some(encoded(rgb, alpha(a)))
}

/// `lab()`, or `lch()` if `lch`: lightness 0–100, and a and b (100% = 125) or chroma
/// (100% = 150) and hue.
fn lab(args: &str, lch: bool) -> Option<Rgba> {
    let (c, a, legacy) = channels(args)?;
    if legacy {
        return None;
    }
    let l = scaled(&c[0], 100.0)?.clamp(0.0, 100.0);
    let [x, y] = match lch {
        true => polar(scaled(&c[1], 150.0)?.max(0.0), hue(&c[2])?),
        false => [scaled(&c[1], 125.0)?, scaled(&c[2], 125.0)?],
    };
    Some(encoded(from_lab([l, x, y]), alpha(a)))
}

/// `oklab()`, or `oklch()` if `lch`: lightness 0–1, and a and b (100% = 0.4) or chroma
/// (100% = 0.4) and hue.
fn oklab_color(args: &str, lch: bool) -> Option<Rgba> {
    let (c, a, legacy) = channels(args)?;
    if legacy {
        return None;
    }
    let l = scaled(&c[0], 1.0)?.clamp(0.0, 1.0);
    let [x, y] = match lch {
        true => polar(scaled(&c[1], 0.4)?.max(0.0), hue(&c[2])?),
        false => [scaled(&c[1], 0.4)?, scaled(&c[2], 0.4)?],
    };
    Some(encoded(from_oklab([l, x, y]), alpha(a)))
}

/// `color()`: a predefined color space, its three channels (100% = 1), and an alpha.
fn color(args: &str) -> Option<Rgba> {
    let (space, rest) = args.trim().split_once(char::is_whitespace)?;
    let (c, a, legacy) = channels(rest)?;
    if legacy {
        return None;
    }
    let v = [
        scaled(&c[0], 1.0)?,
        scaled(&c[1], 1.0)?,
        scaled(&c[2], 1.0)?,
    ];
    let curve = |f: fn(f64) -> f64| v.map(|c| f(c.abs()).copysign(c));
    let rgb = match space {
        "srgb" => v,
        "srgb-linear" => v.map(gamma),
        "display-p3" => from_xyz(times(P3, v.map(linear))),
        "rec2020" => from_xyz(times(REC2020, curve(rec2020))),
        "a98-rgb" => from_xyz(times(A98, curve(|c| c.powf(563.0 / 256.0)))),
        "prophoto-rgb" => {
            let lit = curve(|c| {
                if c <= 16.0 / 512.0 {
                    c / 16.0
                } else {
                    c.powf(1.8)
                }
            });
            from_xyz(times(D50_TO_D65, times(PROPHOTO, lit)))
        }
        "xyz" | "xyz-d65" => from_xyz(v),
        "xyz-d50" => from_xyz(times(D50_TO_D65, v)),
        _ => return None,
    };
    Some(encoded(rgb, alpha(a)))
}

/// A Rec. 2020 channel in linear light.
fn rec2020(c: f64) -> f64 {
    const ALPHA: f64 = 1.09929682680944;
    const BETA: f64 = 0.018053968510807;
    match c < BETA * 4.5 {
        true => c / 4.5,
        false => ((c + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45),
    }
}

/// CIE XYZ, D65 white, of linear-light Display P3, Rec. 2020 and Adobe RGB (1998); and D50
/// white, of linear-light ProPhoto RGB.
const P3: [[f64; 3]; 3] = [
    [0.4865709486482162, 0.26566769316909306, 0.1982172852343625],
    [0.2289745640697488, 0.6917385218365064, 0.079286914093745],
    [0.0, 0.04511338185890264, 1.043944368900976],
];
const REC2020: [[f64; 3]; 3] = [
    [0.6369580483012914, 0.14461690358620832, 0.1688809751641721],
    [0.2627002120112671, 0.6779980715188708, 0.05930171646986196],
    [0.0, 0.028072693049087428, 1.060985057710791],
];
const A98: [[f64; 3]; 3] = [
    [0.5766690429101305, 0.1855582379065463, 0.1882286462349947],
    [0.29734497525053605, 0.6273635662554661, 0.07529145849399788],
    [0.02703136138641234, 0.07068885253582723, 0.9913375368376388],
];
const PROPHOTO: [[f64; 3]; 3] = [
    [0.7977666449006423, 0.13518129740053308, 0.0313477341283922],
    [0.2880748288194013, 0.711835234241873, 0.00008993693872564],
    [0.0, 0.0, 0.8251046025104602],
];

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
            "hsl(none, 10%, 10%)",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn modern_color_functions_draw_as_chrome_draws_them() {
        // Chrome 156's pixels, out-of-gamut channels clipped.
        for (css, chrome) in [
            ("oklch(0.7 0.1 200)", [64, 177, 183]),
            ("oklch(0.7 0.4 30)", [255, 0, 0]),
            ("oklch(0.9 0.3 140)", [75, 255, 0]),
            ("oklch(0.7 none 200)", [158, 158, 158]),
            ("oklch(1.2 0 0)", [255, 255, 255]),
            ("oklab(0.5 0.1 -0.1)", [129, 69, 154]),
            ("oklab(50% 25% -25%)", [129, 69, 154]),
            ("oklab(0.6 0.3 0.3)", [255, 0, 0]),
            ("lab(50 40 -20)", [171, 90, 154]),
            ("lab(50% 32% -16%)", [171, 90, 154]),
            ("lab(120 0 0)", [255, 255, 255]),
            ("lch(60 50 120)", [118, 156, 62]),
            ("lch(60 120 120)", [23, 169, 0]),
            ("lch(50 200 300)", [0, 17, 255]),
            ("hwb(200 20% 30%)", [51, 136, 179]),
            ("hwb(0 60% 60%)", [128, 128, 128]),
            ("color(display-p3 1 0 0)", [255, 0, 0]),
            ("color(srgb 0.2 0.4 0.6)", [51, 102, 153]),
            ("color(srgb 1.5 -0.5 0.5)", [255, 0, 128]),
            ("color(srgb-linear 0.2 0.4 0.6)", [124, 170, 203]),
            ("color(rec2020 0.5 0.3 0.1)", [163, 83, 25]),
            ("color(a98-rgb 0.5 0.3 0.1)", [143, 75, 12]),
            ("color(prophoto-rgb 0.5 0.3 0.1)", [187, 78, 0]),
            ("color(xyz 0.2 0.3 0.4)", [0, 167, 164]),
            ("color(xyz-d65 0.2 0.3 0.4)", [0, 167, 164]),
            ("color(xyz-d50 0.2 0.3 0.4)", [0, 168, 189]),
        ] {
            let ours = bytes(css);
            let near = (0..3).all(|i| ours[i].abs_diff(chrome[i]) <= 1);
            assert!(near && ours[3] == 255, "{css}: {ours:?}, Chrome {chrome:?}");
        }
        assert_eq!(bytes("oklch(0.5 0.1 200 / 50%)")[3], 128);
        assert_eq!(bytes("hwb(0 0 0 / none)")[3], 0);
        for bad in [
            "oklch(0.5, 0.1, 200)",
            "lab(50 0)",
            "lch(50 10 1px)",
            "hwb(0, 10%, 10%)",
            "color(foo 1 0 0)",
            "color(srgb 1 0)",
            "color(srgb 1px 0 0)",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
