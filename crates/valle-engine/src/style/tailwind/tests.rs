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
    ];
    for (name, expected) in cases {
        assert_eq!(css(name), expected, "{name}");
    }
}

#[test]
fn classes_tailwind_does_not_make_are_errors() {
    for name in [
        "p-foo", "p-0.3", "p-4/50", "w-", "p-[]", "m-[a;b]", "-p-4", "foo",
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
    assert!(!valid("a;b") && valid("'a;b'") && !valid("a)"));
}

#[test]
fn the_theme_has_tailwinds_variables() {
    assert_eq!(THEME.value("--spacing"), Some("0.25rem"));
    let sans = THEME.value("--font-sans").unwrap();
    assert!(
        sans.starts_with("-apple-system, BlinkMacSystemFont, 'Segoe UI'"),
        "{sans}"
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
