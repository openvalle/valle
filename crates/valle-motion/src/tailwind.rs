//! Valle Motion admitted Tailwind catalog.
//!
//! Takumi intentionally treats unknown utilities as absent. Motion cannot use that behavior:
//! authoring mistakes would otherwise become valid artifacts with silently different pixels.
//! This module is therefore the backend-neutral admission gate shared by compiler,
//! artifact validation, Native layout, and Web layout.

/// Exact catalog identity. It is also part of the artifact capability set.
pub const TAILWIND_CATALOG: &str = "tailwind";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailwindClassError {
    /// Legal Tailwind surface that Motion deliberately forbids because it is time-, viewport-, or
    /// interaction-dependent.
    Forbidden,
    /// Tailwind utility not present in this catalog.
    Unsupported,
}

/// Validate one already whitespace-separated `className` token.
pub fn validate_tailwind_class(class_name: &str) -> Result<(), TailwindClassError> {
    if class_name.is_empty()
        || class_name.contains(':')
        || class_name.starts_with('!')
        || class_name.ends_with('!')
        || class_name.starts_with("animate-")
        || class_name.starts_with("transition-")
        || matches!(class_name, "transition" | "transform-gpu" | "transform-cpu")
    {
        return Err(TailwindClassError::Forbidden);
    }

    if FIXED.contains(&class_name)
        || is_spacing(class_name)
        || is_sizing(class_name)
        || is_color(class_name)
        || is_typography(class_name)
        || is_border(class_name)
        || is_gradient(class_name)
        || is_opacity(class_name)
        || is_grid(class_name)
        || is_line_clamp(class_name)
    {
        Ok(())
    } else {
        Err(TailwindClassError::Unsupported)
    }
}

const FIXED: &[&str] = &[
    "absolute",
    "block",
    "box-border",
    "box-content",
    "break-all",
    "break-keep",
    "break-normal",
    "capitalize",
    "fixed",
    "flex",
    "flex-auto",
    "flex-col",
    "flex-col-reverse",
    "flex-initial",
    "flex-none",
    "flex-nowrap",
    "flex-row",
    "flex-row-reverse",
    "flex-wrap",
    "flex-wrap-reverse",
    "grid",
    "grid-flow-col",
    "grid-flow-col-dense",
    "grid-flow-dense",
    "grid-flow-row",
    "grid-flow-row-dense",
    "hidden",
    "inline",
    "inline-block",
    "inline-flex",
    "inline-grid",
    "invisible",
    "isolate",
    "isolation-auto",
    "italic",
    "line-through",
    "lowercase",
    "no-underline",
    "normal-case",
    "not-italic",
    "overline",
    "relative",
    "static",
    "text-clip",
    "text-ellipsis",
    "tabular-nums",
    "truncate",
    "underline",
    "uppercase",
    "visible",
    "whitespace-normal",
    "whitespace-nowrap",
    "whitespace-pre",
    "whitespace-pre-line",
    "whitespace-pre-wrap",
    // Alignment and overflow use closed keyword parsers in Takumi.
    "content-around",
    "content-between",
    "content-center",
    "content-end",
    "content-evenly",
    "content-normal",
    "content-start",
    "content-stretch",
    "items-baseline",
    "items-center",
    "items-end",
    "items-start",
    "items-stretch",
    "justify-around",
    "justify-between",
    "justify-center",
    "justify-end",
    "justify-evenly",
    "justify-normal",
    "justify-start",
    "justify-stretch",
    "overflow-auto",
    "overflow-clip",
    "overflow-hidden",
    "overflow-scroll",
    "overflow-visible",
    "self-auto",
    "self-baseline",
    "self-center",
    "self-end",
    "self-start",
    "self-stretch",
];

fn is_line_clamp(class_name: &str) -> bool {
    class_name == "line-clamp-none"
        || class_name
            .strip_prefix("line-clamp-")
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|value| (1..=64).contains(&value))
}

fn is_spacing(class_name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "gap", "gap-x", "gap-y", "m", "mx", "my", "mt", "mr", "mb", "ml", "ms", "me", "p", "px",
        "py", "pt", "pr", "pb", "pl", "ps", "pe", "inset", "inset-x", "inset-y", "top", "right",
        "bottom", "left",
    ];
    let (negative, token) = class_name
        .strip_prefix('-')
        .map_or((false, class_name), |rest| (true, rest));
    PREFIXES.iter().any(|prefix| {
        token
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(|value| {
                let allows_auto = matches!(
                    *prefix,
                    "m" | "mx"
                        | "my"
                        | "mt"
                        | "mr"
                        | "mb"
                        | "ml"
                        | "ms"
                        | "me"
                        | "inset"
                        | "inset-x"
                        | "inset-y"
                        | "top"
                        | "right"
                        | "bottom"
                        | "left"
                );
                let allows_negative = !prefix.starts_with('p') && !prefix.starts_with("gap");
                (!negative || allows_negative)
                    && ((allows_auto && value == "auto")
                        || value == "px"
                        || value == "full"
                        || non_negative_number(value))
            })
    })
}

fn is_sizing(class_name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "w", "h", "min-w", "min-h", "max-w", "max-h", "size", "basis",
    ];
    PREFIXES.iter().any(|prefix| {
        class_name
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(|value| {
                matches!(
                    value,
                    "auto" | "px" | "full" | "screen" | "min" | "max" | "fit"
                ) || non_negative_number(value)
            })
    }) || matches!(class_name, "aspect-auto" | "aspect-square" | "aspect-video")
}

fn is_color(class_name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "bg",
        "text",
        "border",
        "border-t",
        "border-r",
        "border-b",
        "border-l",
        "border-x",
        "border-y",
        "outline",
        "decoration",
        "shadow",
        "text-shadow",
    ];
    PREFIXES.iter().any(|prefix| {
        class_name
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(is_color_token)
    })
}

fn is_color_token(token: &str) -> bool {
    if matches!(token, "black" | "white" | "transparent" | "current") {
        return true;
    }
    let (base, alpha) = token
        .split_once('/')
        .map_or((token, None), |(base, alpha)| (base, Some(alpha)));
    if alpha.is_some_and(|value| !percentage(value)) {
        return false;
    }
    let Some((family, shade)) = base.rsplit_once('-') else {
        return false;
    };
    const FAMILIES: &[&str] = &[
        "slate", "gray", "zinc", "neutral", "stone", "red", "orange", "amber", "yellow", "lime",
        "green", "emerald", "teal", "cyan", "sky", "blue", "indigo", "violet", "purple", "fuchsia",
        "pink", "rose",
    ];
    const SHADES: &[&str] = &[
        "50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950",
    ];
    FAMILIES.contains(&family) && SHADES.contains(&shade)
}

fn is_typography(class_name: &str) -> bool {
    const FONT_SIZES: &[&str] = &[
        "text-xs",
        "text-sm",
        "text-base",
        "text-lg",
        "text-xl",
        "text-2xl",
        "text-3xl",
        "text-4xl",
        "text-5xl",
        "text-6xl",
        "text-7xl",
        "text-8xl",
        "text-9xl",
    ];
    const TEXT_ALIGN: &[&str] = &[
        "text-left",
        "text-center",
        "text-right",
        "text-justify",
        "text-start",
        "text-end",
    ];
    const FONT_WEIGHT: &[&str] = &[
        "font-thin",
        "font-extralight",
        "font-light",
        "font-normal",
        "font-medium",
        "font-semibold",
        "font-bold",
        "font-extrabold",
        "font-black",
    ];
    const TRACKING: &[&str] = &[
        "tracking-tighter",
        "tracking-tight",
        "tracking-normal",
        "tracking-wide",
        "tracking-wider",
        "tracking-widest",
    ];
    FONT_SIZES.contains(&class_name)
        || TEXT_ALIGN.contains(&class_name)
        || FONT_WEIGHT.contains(&class_name)
        || TRACKING.contains(&class_name)
        || class_name.strip_prefix("leading-").is_some_and(|value| {
            non_negative_number(value)
                || matches!(
                    value,
                    "none" | "tight" | "snug" | "normal" | "relaxed" | "loose"
                )
        })
}

fn is_border(class_name: &str) -> bool {
    const ROUNDED_PREFIXES: &[&str] = &[
        "rounded",
        "rounded-t",
        "rounded-r",
        "rounded-b",
        "rounded-l",
        "rounded-tl",
        "rounded-tr",
        "rounded-br",
        "rounded-bl",
    ];
    const RADII: &[&str] = &[
        "none", "xs", "sm", "md", "lg", "xl", "2xl", "3xl", "4xl", "full",
    ];
    if class_name == "rounded" {
        return true;
    }
    if ROUNDED_PREFIXES.iter().any(|prefix| {
        class_name
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(|value| RADII.contains(&value))
    }) {
        return true;
    }
    if matches!(
        class_name,
        "border"
            | "outline"
            | "border-solid"
            | "border-dashed"
            | "border-dotted"
            | "border-double"
            | "border-none"
            | "outline-solid"
            | "outline-dashed"
            | "outline-dotted"
            | "outline-double"
            | "outline-none"
    ) {
        return true;
    }
    const WIDTH_PREFIXES: &[&str] = &[
        "border",
        "border-t",
        "border-r",
        "border-b",
        "border-l",
        "border-x",
        "border-y",
        "outline",
        "outline-offset",
    ];
    WIDTH_PREFIXES.iter().any(|prefix| {
        class_name
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .is_some_and(non_negative_number)
    }) || matches!(
        class_name,
        "shadow"
            | "shadow-2xs"
            | "shadow-xs"
            | "shadow-sm"
            | "shadow-md"
            | "shadow-lg"
            | "shadow-xl"
            | "shadow-2xl"
            | "shadow-none"
    )
}

fn is_opacity(class_name: &str) -> bool {
    class_name.strip_prefix("opacity-").is_some_and(percentage)
}

/// Explicit Tailwind v4 gradient catalog; successful parsing alone does not admit arbitrary
/// utilities.
fn is_gradient(class_name: &str) -> bool {
    const LINEAR_DIRECTIONS: &[&str] = &[
        "bg-linear-to-t",
        "bg-linear-to-tr",
        "bg-linear-to-r",
        "bg-linear-to-br",
        "bg-linear-to-b",
        "bg-linear-to-bl",
        "bg-linear-to-l",
        "bg-linear-to-tl",
    ];
    if LINEAR_DIRECTIONS.contains(&class_name) {
        return true;
    }
    if matches!(class_name, "bg-radial" | "bg-conic") {
        return true;
    }
    for prefix in ["from-", "via-", "to-"] {
        if let Some(value) = class_name.strip_prefix(prefix) {
            return is_color_token(value) || value.strip_suffix('%').is_some_and(percentage);
        }
    }
    false
}

fn is_grid(class_name: &str) -> bool {
    if matches!(
        class_name,
        "grid-cols-none"
            | "grid-rows-none"
            | "col-span-full"
            | "row-span-full"
            | "col-start-auto"
            | "col-end-auto"
            | "row-start-auto"
            | "row-end-auto"
    ) {
        return true;
    }
    const COUNT_PREFIXES: &[&str] = &[
        "grid-cols",
        "grid-rows",
        "col-span",
        "row-span",
        "col-start",
        "col-end",
        "row-start",
        "row-end",
    ];
    COUNT_PREFIXES.iter().any(|prefix| {
        class_name
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .and_then(|value| value.parse::<u32>().ok())
            .is_some_and(|value| (1..=64).contains(&value))
    }) || [
        "auto-cols-auto",
        "auto-cols-min",
        "auto-cols-max",
        "auto-cols-fr",
        "auto-rows-auto",
        "auto-rows-min",
        "auto-rows-max",
        "auto-rows-fr",
    ]
    .contains(&class_name)
}

fn non_negative_number(value: &str) -> bool {
    value
        .parse::<f32>()
        .is_ok_and(|number| number.is_finite() && number >= 0.0)
}

fn percentage(value: &str) -> bool {
    value
        .parse::<f32>()
        .is_ok_and(|number| number.is_finite() && (0.0..=100.0).contains(&number))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_versioned_layout_grid_and_visual_catalog() {
        for class_name in [
            "flex",
            "items-center",
            "gap-4",
            "p-8",
            "grid",
            "grid-cols-3",
            "col-span-2",
            "bg-slate-950",
            "text-white",
            "text-4xl",
            "font-bold",
            "rounded-xl",
            "bg-linear-to-r",
            "bg-radial",
            "bg-conic",
            "border-dashed",
            "border-dotted",
            "border-double",
            "outline-dashed",
            "from-cyan-400",
            "via-blue-500",
            "to-violet-500",
            "from-10%",
            "to-90%",
            "opacity-75",
        ] {
            assert_eq!(validate_tailwind_class(class_name), Ok(()), "{class_name}");
        }
    }

    #[test]
    fn rejects_variants_animation_and_unknown_utilities() {
        for class_name in [
            "sm:grid",
            "hover:bg-red-500",
            "animate-spin",
            "transition",
            "!p-4",
        ] {
            assert_eq!(
                validate_tailwind_class(class_name),
                Err(TailwindClassError::Forbidden),
                "{class_name}"
            );
        }
        for class_name in [
            "gird",
            "grid-cols-0",
            "bg-brand-500",
            "bg-conic-45",
            "made-up-4",
        ] {
            assert_eq!(
                validate_tailwind_class(class_name),
                Err(TailwindClassError::Unsupported),
                "{class_name}"
            );
        }
    }
}
