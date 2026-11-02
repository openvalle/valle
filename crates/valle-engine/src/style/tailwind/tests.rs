use super::*;

fn css(name: &str) -> String {
    print(&class(name).unwrap())
}

fn error(name: &str) -> String {
    class(name).unwrap_err()
}

#[test]
fn classes_expand_to_tailwinds_css() {
    let cases = [
        ("p-4", "& { padding: calc(var(--spacing) * 4); }"),
        ("p-1", "& { padding: var(--spacing); }"),
        ("inset-x-0", "& { inset-inline: 0px; }"),
        ("-m-2", "& { margin: calc(var(--spacing) * -2); }"),
        ("p-0.75", "& { padding: calc(var(--spacing) * 0.75); }"),
        ("w-1/2", "& { width: calc(1 / 2 * 100%); }"),
        ("-inset-1/2", "& { inset: calc(calc(1 / 2 * 100%) * -1); }"),
        ("w-px", "& { width: 1px; }"),
        ("-top-px", "& { top: -1px; }"),
        ("max-w-md", "& { max-width: var(--container-md); }"),
        ("z-10", "& { z-index: 10; }"),
        ("-z-10", "& { z-index: calc(10 * -1); }"),
        ("aspect-3/2", "& { aspect-ratio: 3/2; }"),
        ("aspect-video", "& { aspect-ratio: var(--aspect-video); }"),
        (
            "size-4",
            "& { width: calc(var(--spacing) * 4); height: calc(var(--spacing) * 4); }",
        ),
        ("p-[3px]", "& { padding: 3px; }"),
        ("w-[calc(100%-2rem)]", "& { width: calc(100% - 2rem); }"),
        ("[padding:3px_1px]", "& { padding: 3px 1px; }"),
        (
            "p-4!",
            "& { padding: calc(var(--spacing) * 4) !important; }",
        ),
        ("overflow-x-clip", "& { overflow-x: clip; }"),
        (
            "space-x-4",
            ":where(& > :not(:last-child)) { --tw-space-x-reverse: 0; \
             margin-inline-start: calc(calc(var(--spacing) * 4) * var(--tw-space-x-reverse)); \
             margin-inline-end: calc(calc(var(--spacing) * 4) * calc(1 - \
             var(--tw-space-x-reverse))); }",
        ),
        (
            "space-y-0",
            ":where(& > :not(:last-child)) { --tw-space-y-reverse: 0; margin-block-start: 0; \
             margin-block-end: 0; }",
        ),
        (
            "space-x-reverse",
            ":where(& > :not(:last-child)) { --tw-space-x-reverse: 1; }",
        ),
        ("order-first", "& { order: -9999; }"),
        ("-order-2", "& { order: calc(2 * -1); }"),
        ("col-span-2", "& { grid-column: span 2 / span 2; }"),
        ("row-span-full", "& { grid-row: 1 / -1; }"),
        ("-col-start-2", "& { grid-column-start: calc(2 * -1); }"),
        ("flex-1", "& { flex: 1; }"),
        ("flex-1/2", "& { flex: calc(1/2 * 100%); }"),
        ("flex-[2_2_0%]", "& { flex: 2 2 0%; }"),
        ("grow", "& { flex-grow: 1; }"),
        ("shrink-0", "& { flex-shrink: 0; }"),
        ("basis-md", "& { flex-basis: var(--container-md); }"),
        ("basis-1/3", "& { flex-basis: calc(1 / 3 * 100%); }"),
        (
            "grid-cols-3",
            "& { grid-template-columns: repeat(3, minmax(0, 1fr)); }",
        ),
        ("auto-rows-fr", "& { grid-auto-rows: minmax(0, 1fr); }"),
        (
            "auto-cols-4",
            "& { grid-auto-columns: calc(var(--spacing) * 4); }",
        ),
        ("grid-flow-col-dense", "& { grid-auto-flow: column dense; }"),
        ("justify-self-end", "& { justify-self: flex-end; }"),
        (
            "text-sm",
            "& { font-size: var(--text-sm); line-height: var(--tw-leading, \
             var(--text-sm--line-height)); }",
        ),
        (
            "text-sm/6",
            "& { font-size: var(--text-sm); line-height: calc(var(--spacing) * 6); }",
        ),
        (
            "text-[13px]/[20px]",
            "& { font-size: 13px; line-height: 20px; }",
        ),
        ("text-[1.5em]", "& { font-size: 1.5em; }"),
        (
            "text-red-500/50",
            "& { color: color-mix(in oklab, var(--color-red-500) 50%, transparent); }",
        ),
        ("text-current", "& { color: currentcolor; }"),
        ("text-[#f00]", "& { color: #f00; }"),
        (
            "font-bold",
            "& { --tw-font-weight: var(--font-weight-bold); font-weight: \
             var(--font-weight-bold); }",
        ),
        ("font-sans", "& { font-family: var(--font-sans); }"),
        (
            "font-[300]",
            "& { --tw-font-weight: 300; font-weight: 300; }",
        ),
        ("font-['Brand_Sans']", "& { font-family: 'Brand Sans'; }"),
        (
            "leading-tight",
            "& { --tw-leading: var(--leading-tight); line-height: var(--leading-tight); }",
        ),
        ("leading-none", "& { --tw-leading: 1; line-height: 1; }"),
        (
            "-tracking-[1px]",
            "& { --tw-tracking: calc(1px * -1); letter-spacing: calc(1px * -1); }",
        ),
        ("decoration-2", "& { text-decoration-thickness: 2px; }"),
        (
            "decoration-sky-500",
            "& { text-decoration-color: var(--color-sky-500); }",
        ),
        (
            "-underline-offset-2",
            "& { text-underline-offset: calc(2px * -1); }",
        ),
        ("font-stretch-75%", "& { font-stretch: 75%; }"),
        (
            "tabular-nums",
            "& { --tw-numeric-spacing: tabular-nums; font-variant-numeric: var(--tw-ordinal,) \
             var(--tw-slashed-zero,) var(--tw-numeric-figure,) var(--tw-numeric-spacing,) \
             var(--tw-numeric-fraction,); }",
        ),
        (
            "line-clamp-3",
            "& { overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; \
             -webkit-line-clamp: 3; }",
        ),
    ];
    for (name, expected) in cases {
        assert_eq!(css(name), expected, "{name}");
    }
}

#[test]
fn classes_tailwind_does_not_make_are_errors() {
    for name in [
        "p-foo",
        "p-0.3",
        "p-4/50",
        "w-",
        "p-[]",
        "m-[a;b]",
        "-p-4",
        "foo",
        "grid-cols-0",
        "-col-span-2",
        "grow-[2]/3",
    ] {
        assert_eq!(
            error(name),
            format!("className {name} is not a Tailwind class, or not supported yet")
        );
    }
}

#[test]
fn errors_say_what_to_write_instead() {
    let e = error("hover:p-4");
    assert!(
        e.starts_with("className hover:p-4: variants are not supported: "),
        "{e}"
    );
    assert_eq!(
        error("!p-4"),
        "className !p-4: Tailwind v4 marks a class important at its end: write p-4!"
    );
    for name in ["w-(--size)", "[--gap:4px]", "p-4/(--a)"] {
        assert_eq!(
            error(name),
            format!(
                "className {name}: CSS variables are not supported: name the value with a const"
            )
        );
    }
    for (v3, v4) in [
        ("bg-gradient-to-r", "bg-linear-to-r"),
        ("flex-grow", "grow"),
        ("flex-shrink-0", "shrink-0"),
        ("-start-4", "-inset-s-4"),
        ("rounded-t", "rounded-t-sm"),
        ("max-w-screen-md", "max-w-[48rem]"),
        ("max-w-prose", "max-w-[65ch]"),
    ] {
        assert_eq!(
            error(v3),
            format!("className {v3} is a Tailwind v3 name: write {v4}")
        );
    }
    // What the engine cannot draw fails as the style would.
    assert_eq!(
        error("w-[var(--size)]"),
        "className w-[var(--size)]: width: CSS variables are not supported: name the value with a \
         const"
    );
    assert_eq!(
        error("sticky"),
        "className sticky: position: sticky is not supported: the canvas does not scroll; use \
         relative or absolute"
    );
    assert_eq!(
        error("float-left"),
        "className float-left: float is not supported: there is no document flow; use flex or \
         grid layout"
    );
    assert!(error("items-center-safe").starts_with("className items-center-safe: align-items: "));
    assert_eq!(
        error("align-middle"),
        "className align-middle: vertical-align is not supported: there is no HTML inline flow; \
         use flex layout, and Span for styled runs of text"
    );
    let e = error("container");
    assert_eq!(
        e,
        "className container: its CSS has @media rules, which do not apply: the canvas has a \
         fixed size"
    );
}

/// The declarations of an element of `own` classes, the child but the last of one of `parent`
/// classes.
fn applied(own: &[&str], parent: &[&str]) -> Vec<(String, String, bool)> {
    let classes = |names: &[&str]| names.iter().map(|n| class(n).unwrap()).collect::<Vec<_>>();
    let declarations = element(&classes(own), &classes(parent)).unwrap();
    (declarations.into_iter())
        .map(|d| (d.property, d.value, d.important))
        .collect()
}

fn d(property: &str, value: &str, important: bool) -> (String, String, bool) {
    (property.into(), value.into(), important)
}

#[test]
fn variables_resolve_on_each_element() {
    assert_eq!(
        applied(&["p-4", "max-w-md"], &[]),
        [
            d("max-width", "28rem", false),
            d("padding", "calc(0.25rem * 4)", false)
        ]
    );
    // The parent's spacing, reversed by its other class, which comes later.
    assert_eq!(
        applied(&[], &["space-x-reverse", "space-x-2"]),
        [
            d("margin-inline-start", "calc(calc(0.25rem * 2) * 1)", false),
            d(
                "margin-inline-end",
                "calc(calc(0.25rem * 2) * calc(1 - 1))",
                false
            )
        ]
    );
}

#[test]
fn text_classes_read_each_others_variables() {
    // The size's line height gives way to the leading's, whatever the order written.
    assert_eq!(
        applied(&["leading-6", "text-sm"], &[]),
        [
            d("font-size", "0.875rem", false),
            d("line-height", "calc(0.25rem * 6)", false),
            d("line-height", "calc(0.25rem * 6)", false)
        ]
    );
    assert_eq!(
        applied(&["text-sm"], &[])[1],
        d("line-height", "calc(1.25 / 0.875)", false)
    );
    // Numeric variants add up.
    let numeric = applied(&["ordinal", "tabular-nums"], &[]);
    let words: Vec<&str> = numeric[1].1.split_whitespace().collect();
    assert_eq!(words, ["ordinal", "tabular-nums"]);
}

#[test]
fn classes_cascade_in_tailwinds_order_and_important_ones_last() {
    // `padding` comes before `padding-inline` whatever the order written.
    assert_eq!(
        applied(&["px-2", "p-4"], &[]),
        [
            d("padding", "calc(0.25rem * 4)", false),
            d("padding-inline", "calc(0.25rem * 2)", false)
        ]
    );
    // By name, numbers by their values, where the properties are the same.
    assert_eq!(
        applied(&["p-10", "p-2!", "p-3"], &[]),
        [
            d("padding", "calc(0.25rem * 3)", false),
            d("padding", "calc(0.25rem * 10)", false),
            d("padding", "calc(0.25rem * 2)", true)
        ]
    );
    // The parent's rules for its children are less specific than the child's own.
    assert_eq!(
        applied(&["ms-1"], &["space-x-4"])[2],
        d("margin-inline-start", "0.25rem", false)
    );
}

#[test]
fn rules_sort_by_their_first_property_then_by_how_many_they_set() {
    let order = |a: &str, b: &str| compare(&class(a).unwrap(), &class(b).unwrap());
    assert_eq!(order("p-4", "px-4"), Ordering::Less);
    assert_eq!(order("m-4", "p-4"), Ordering::Less);
    assert_eq!(order("p-2", "p-10"), Ordering::Less);
    assert_eq!(order("p-1.5", "p-1"), Ordering::Greater);
    assert_eq!(natural("a10b", "a9b"), Ordering::Greater);
    assert_eq!(natural("a-1", "a-1.5"), Ordering::Less);
}

#[test]
fn arbitrary_values_decode_as_tailwind_does() {
    assert_eq!(decode("1px_solid"), "1px solid");
    assert_eq!(decode("a\\_b"), "a_b");
    assert_eq!(decode("url(a_b.png)"), "url(a_b.png)");
    assert_eq!(decode("var(--my_size,1_px)"), "var(--my_size,1 px)");
    assert_eq!(decode("calc(100%-2rem*2)"), "calc(100% - 2rem * 2)");
    assert_eq!(decode("min(1px,2px)"), "min(1px, 2px)");
    assert_eq!(decode("calc(1e-3*2)"), "calc(1e-3 * 2)");
    assert_eq!(segment("a:[b:c]:d", b':'), ["a", "[b:c]", "d"]);
    let kinds = ["color", "length", "percentage", "relative-size"];
    for (value, kind) in [
        ("larger", Some("relative-size")),
        ("1e3px", Some("length")),
        ("calc(1px+2%)", Some("length")),
        ("50%", Some("percentage")),
        ("RGB(0 0 0)", Some("color")),
        ("Red", Some("color")),
        ("var(--x)", None),
        ("1.px", None),
    ] {
        assert_eq!(types::infer(value, &kinds), kind, "{value}");
    }
    assert!(!valid("a;b") && valid("'a;b'") && !valid("a)"));
}

#[test]
fn the_theme_has_tailwinds_variables() {
    assert_eq!(THEME.value("--spacing"), Some("0.25rem"));
    assert_eq!(
        THEME.value("--text-sm--line-height"),
        Some("calc(1.25 / 0.875)")
    );
    // Valle's font stacks, its built-in families.
    assert_eq!(THEME.value("--font-sans"), Some("sans-serif"));
    assert_eq!(THEME.value("--font-mono"), Some("monospace"));
    assert_eq!(
        THEME.resolve_with("sm", &["--text"], &["--line-height", "--font-weight"]),
        Some((
            "var(--text-sm)".into(),
            vec![Some("var(--text-sm--line-height)".into()), None]
        ))
    );
    // Deprecated values are written as they are.
    assert_eq!(
        THEME.resolve(None, &["--radius"]).as_deref(),
        Some("0.25rem")
    );
    assert_eq!(
        THEME.resolve(Some("md"), &["--radius"]).as_deref(),
        Some("var(--radius-md)")
    );
    // A namespace's keys that are another's.
    assert_eq!(THEME.resolve(Some("weight-bold"), &["--font"]), None);
    assert_eq!(
        THEME.value("--animate-spin"),
        Some("spin 1s linear infinite")
    );
}
