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
        (
            "bg-sky-500",
            "& { background-color: var(--color-sky-500); }",
        ),
        (
            "bg-black/[0.3]",
            "& { background-color: color-mix(in oklab, var(--color-black) 30%, transparent); }",
        ),
        ("bg-[#0af]", "& { background-color: #0af; }"),
        ("bg-[center_top]", "& { background-position: center top; }"),
        ("bg-[10px]", "& { background-position: 10px; }"),
        (
            "bg-[length:200px_100px]",
            "& { background-size: 200px 100px; }",
        ),
        (
            "bg-[linear-gradient(red,blue)]",
            "& { background-image: linear-gradient(red,blue); }",
        ),
        ("fill-none", "& { fill: none; }"),
        (
            "fill-red-500/50",
            "& { fill: color-mix(in oklab, var(--color-red-500) 50%, transparent); }",
        ),
        ("stroke-current", "& { stroke: currentcolor; }"),
        ("stroke-2", "& { stroke-width: 2; }"),
        ("stroke-[1.5]", "& { stroke-width: 1.5; }"),
        ("stroke-[#f00]", "& { stroke: #f00; }"),
        ("bg-top-left", "& { background-position: left top; }"),
        ("bg-size-[auto_50%]", "& { background-size: auto 50%; }"),
        ("bg-repeat-round", "& { background-repeat: round; }"),
        (
            "bg-linear-to-r",
            "& { --tw-gradient-position: to right; @supports (background-image: \
             linear-gradient(in lab, red, red)) { --tw-gradient-position: to right in oklab; } \
             background-image: linear-gradient(var(--tw-gradient-stops)); }",
        ),
        (
            "-bg-linear-45/longer",
            "& { --tw-gradient-position: calc(45deg * -1); @supports (background-image: \
             linear-gradient(in lab, red, red)) { --tw-gradient-position: calc(45deg * -1) in \
             oklch longer hue; } background-image: linear-gradient(var(--tw-gradient-stops)); }",
        ),
        (
            "bg-linear-[25deg,red_5%,blue]",
            "& { --tw-gradient-position: 25deg,red 5%,blue; background-image: \
             linear-gradient(var(--tw-gradient-stops,25deg,red 5%,blue)); }",
        ),
        (
            "bg-conic-90/srgb",
            "& { --tw-gradient-position: from 90deg in srgb; background-image: \
             conic-gradient(var(--tw-gradient-stops)); }",
        ),
        (
            "bg-radial-[at_25%_25%]",
            "& { --tw-gradient-position: at 25% 25%; background-image: \
             radial-gradient(var(--tw-gradient-stops,at 25% 25%)); }",
        ),
        (
            "from-red-500",
            "& { --tw-gradient-from: var(--color-red-500); --tw-gradient-stops: \
             var(--tw-gradient-via-stops, var(--tw-gradient-position), var(--tw-gradient-from) \
             var(--tw-gradient-from-position), var(--tw-gradient-to) \
             var(--tw-gradient-to-position)); }",
        ),
        ("via-30%", "& { --tw-gradient-via-position: 30%; }"),
        ("to-[3px]", "& { --tw-gradient-to-position: 3px; }"),
        ("rounded-lg", "& { border-radius: var(--radius-lg); }"),
        ("rounded-full", "& { border-radius: calc(infinity * 1px); }"),
        (
            "rounded-t-none",
            "& { border-top-left-radius: 0; border-top-right-radius: 0; }",
        ),
        (
            "border",
            "& { border-style: var(--tw-border-style); border-width: 1px; }",
        ),
        (
            "border-x-4",
            "& { border-inline-style: var(--tw-border-style); border-inline-width: 4px; }",
        ),
        (
            "border-t-sky-500/50",
            "& { border-top-color: color-mix(in oklab, var(--color-sky-500) 50%, transparent); }",
        ),
        (
            "border-dashed",
            "& { --tw-border-style: dashed; border-style: dashed; }",
        ),
        (
            "divide-y-2",
            ":where(& > :not(:last-child)) { --tw-divide-y-reverse: 0; border-bottom-style: \
             var(--tw-border-style); border-top-style: var(--tw-border-style); border-top-width: \
             calc(2px * var(--tw-divide-y-reverse)); border-bottom-width: calc(2px * calc(1 - \
             var(--tw-divide-y-reverse))); }",
        ),
        (
            "divide-red-500",
            ":where(& > :not(:last-child)) { border-color: var(--color-red-500); }",
        ),
        (
            "outline-2",
            "& { outline-style: var(--tw-outline-style); outline-width: 2px; }",
        ),
        ("-outline-offset-2", "& { outline-offset: calc(2px * -1); }"),
        (
            "outline-hidden",
            "& { --tw-outline-style: none; outline-style: none; @media (forced-colors: active) { \
             outline: 2px solid transparent; outline-offset: 2px; } }",
        ),
        (
            "shadow-md/50",
            "& { --tw-shadow-alpha: 50%; --tw-shadow: 0 4px 6px -1px var(--tw-shadow-color, \
             oklab(from rgb(0 0 0 / 0.1) l a b / 50%)), 0 2px 4px -2px var(--tw-shadow-color, \
             oklab(from rgb(0 0 0 / 0.1) l a b / 50%)); box-shadow: var(--tw-inset-shadow), \
             var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), var(--tw-ring-shadow), \
             var(--tw-shadow); }",
        ),
        (
            "shadow-[0_0_4px]",
            "& { --tw-shadow: 0 0 4px var(--tw-shadow-color, currentcolor); box-shadow: \
             var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), \
             var(--tw-ring-shadow), var(--tw-shadow); }",
        ),
        (
            "shadow-red-500/50",
            "& { --tw-shadow-color: color-mix(in oklab, color-mix(in oklab, var(--color-red-500) \
             50%, transparent) var(--tw-shadow-alpha), transparent); }",
        ),
        (
            "inset-shadow-[0_2px_red]/20",
            "& { --tw-inset-shadow-alpha: 20%; --tw-inset-shadow: inset 0 2px \
             var(--tw-inset-shadow-color, oklab(from red l a b / 20%)); box-shadow: \
             var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), \
             var(--tw-ring-shadow), var(--tw-shadow); }",
        ),
        (
            "ring",
            "& { --tw-ring-shadow: var(--tw-ring-inset,) 0 0 0 calc(1px + \
             var(--tw-ring-offset-width)) var(--tw-ring-color, currentcolor); box-shadow: \
             var(--tw-inset-shadow), var(--tw-inset-ring-shadow), var(--tw-ring-offset-shadow), \
             var(--tw-ring-shadow), var(--tw-shadow); }",
        ),
        (
            "ring-offset-2",
            "& { --tw-ring-offset-width: 2px; --tw-ring-offset-shadow: var(--tw-ring-inset,) 0 0 \
             0 var(--tw-ring-offset-width) var(--tw-ring-offset-color); }",
        ),
        (
            "text-shadow-[0_1px_red]",
            "& { text-shadow: 0 1px var(--tw-text-shadow-color, red); }",
        ),
        ("opacity-40", "& { opacity: 40%; }"),
        ("opacity-[0.3]", "& { opacity: 0.3; }"),
        ("mix-blend-multiply", "& { mix-blend-mode: multiply; }"),
        ("bg-blend-screen", "& { background-blend-mode: screen; }"),
        ("isolate", "& { isolation: isolate; }"),
        (
            "-translate-x-1/2",
            "& { --tw-translate-x: calc(calc(1 / 2 * 100%) * -1); \
             translate: var(--tw-translate-x) var(--tw-translate-y); }",
        ),
        (
            "translate-full",
            "& { --tw-translate-x: 100%; --tw-translate-y: 100%; \
             translate: var(--tw-translate-x) var(--tw-translate-y); }",
        ),
        (
            "scale-50",
            "& { --tw-scale-x: 50%; --tw-scale-y: 50%; --tw-scale-z: 50%; \
             scale: var(--tw-scale-x) var(--tw-scale-y); }",
        ),
        ("-rotate-45", "& { rotate: calc(45deg * -1); }"),
        (
            "rotate-x-45",
            "& { --tw-rotate-x: rotateX(45deg); transform: var(--tw-rotate-x,) \
             var(--tw-rotate-y,) var(--tw-rotate-z,) var(--tw-skew-x,) var(--tw-skew-y,); }",
        ),
        ("origin-top-right", "& { transform-origin: 100% 0; }"),
        ("mask-[50%]", "& { mask-position: 50%; }"),
        (
            "mask-radial-at-top-left",
            "& { --tw-mask-radial-position: top left; }",
        ),
        (
            "mask-t-from-50%",
            "& { mask-image: var(--tw-mask-linear), var(--tw-mask-radial), var(--tw-mask-conic); \
             mask-composite: intersect; --tw-mask-linear: var(--tw-mask-left), \
             var(--tw-mask-right), var(--tw-mask-bottom), var(--tw-mask-top); --tw-mask-top: \
             linear-gradient(to top, var(--tw-mask-top-from-color) \
             var(--tw-mask-top-from-position), var(--tw-mask-top-to-color) \
             var(--tw-mask-top-to-position)); --tw-mask-top-from-position: 50%; }",
        ),
        ("object-cover", "& { object-fit: cover; }"),
        ("object-top-left", "& { object-position: left top; }"),
        ("object-[25%_75%]", "& { object-position: 25% 75%; }"),
        (
            "drop-shadow-[0_0_4px_red]/50",
            "& { --tw-drop-shadow-alpha: 50%; --tw-drop-shadow-size: drop-shadow(0 0 4px \
             var(--tw-drop-shadow-color, oklab(from red l a b / 50%))); --tw-drop-shadow: \
             var(--tw-drop-shadow-size); filter: var(--tw-blur,) var(--tw-brightness,) \
             var(--tw-contrast,) var(--tw-grayscale,) var(--tw-hue-rotate,) var(--tw-invert,) \
             var(--tw-saturate,) var(--tw-sepia,) var(--tw-drop-shadow,); }",
        ),
        (
            "perspective-near",
            "& { perspective: var(--perspective-near); }",
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
        "bg-[10px]/50",
        "bg-foo",
        "stroke-[3px]/50",
        "bg-linear",
        "-bg-linear-[to_right]",
        "-bg-linear-to-r",
        "bg-radial-50",
        "from-31",
        "from-[3px]/50",
        "border-2/50",
        "outline-[3px]/50",
        "rounded-foo",
        "-rounded-lg",
        "divide-x-[3px]/2",
        "shadow-none/50",
        "inset-shadow",
        "text-shadow",
        "ring-2/50",
        "ring-offset-2/50",
        "opacity-0.3",
        "opacity-50/50",
        "scale-50/50",
        "-rotate-[1_1_0]",
        "transform-foo",
        "filter-foo",
        "mask-foo",
        "mask-t-from-foo",
        "mask-l-to-33.3%",
        "-mask-radial-[closest-side]",
        "drop-shadow-sm/foo",
        "brightness-[50%]/50",
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
        ("bg-opacity-50", "bg-<color>/50"),
        ("text-opacity-25", "text-<color>/25"),
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
    assert!(
        error("items-baseline-last").starts_with("className items-baseline-last: align-items: ")
    );
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

#[test]
fn what_no_canvas_takes_is_left_out() {
    assert!(applied(&["antialiased"], &[]).is_empty());
    assert!(applied(&["subpixel-antialiased"], &[]).is_empty());
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
fn borders_take_their_style_and_divide_children() {
    let own = applied(&["border-2", "border-dotted"], &[]);
    assert!(own.contains(&d("border-width", "2px", false)), "{own:?}");
    for (p, v, _) in &own {
        assert!(p != "border-style" || v == "dotted", "{own:?}");
    }
    assert_eq!(
        applied(&[], &["divide-x-2", "divide-x-reverse", "divide-dashed"]),
        [
            d("border-inline-style", "dashed", false),
            d("border-inline-start-width", "calc(2px * 1)", false),
            d("border-inline-end-width", "calc(2px * calc(1 - 1))", false),
            d("border-style", "dashed", false)
        ]
    );
}

#[test]
fn shadows_stack_rings_over_box_shadows_in_their_colors() {
    let shadow = |classes: &[&str]| applied(classes, &[]).pop().unwrap().1;
    // Each kind is none until a class sets it; the offset is the background's, white unless set.
    assert_eq!(
        shadow(&["ring-red-500", "shadow-sm", "ring-offset-2", "ring-2"]),
        "0 0 #0000, 0 0 #0000,  0 0 0 2px #fff,  0 0 0 calc(2px + 2px) oklch(63.7% 0.237 25.331), \
         0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1)"
    );
    // A shadow's color takes the opacity the shadow's modifier gives.
    let red = "color-mix(in oklab, oklch(63.7% 0.237 25.331) 50%, transparent)";
    assert_eq!(
        shadow(&["shadow-red-500", "shadow-md/50"]),
        format!(
            "0 0 #0000, 0 0 #0000, 0 0 #0000, 0 0 #0000, 0 4px 6px -1px {red}, 0 2px 4px -2px {red}"
        )
    );
}

#[test]
fn transforms_read_each_axis() {
    let last = |classes: &[&str]| applied(classes, &[]).pop();
    assert_eq!(
        last(&["translate-x-full", "-translate-y-1/2"]),
        Some(d("translate", "100% calc(calc(1 / 2 * 100%) * -1)", false))
    );
    assert_eq!(
        last(&["rotate-y-45", "skew-x-6"]),
        Some(d("transform", " rotateY(45deg)  skewX(6deg) ", false))
    );
    // With no axis set there is nothing to transform.
    assert_eq!(last(&["transform"]), None);
}

#[test]
fn filters_run_in_tailwinds_order_and_drop_shadows_take_their_color() {
    let last = |classes: &[&str]| applied(classes, &[]).pop();
    // Whatever order they are written in, and without the name only Safari reads.
    let filter = last(&["sepia", "blur-sm", "brightness-150"]).unwrap().1;
    assert_eq!(
        filter.split_whitespace().collect::<Vec<_>>(),
        ["blur(8px)", "brightness(150%)", "sepia(100%)"]
    );
    let backdrop = applied(&["backdrop-blur-sm"], &[]);
    assert_eq!(backdrop.len(), 1);
    assert_eq!(backdrop[0].0, "backdrop-filter");
    let red = "color-mix(in oklab, oklch(63.7% 0.237 25.331) 50%, transparent)";
    let filter = last(&["drop-shadow-md/50", "drop-shadow-red-500"]);
    assert_eq!(
        filter.unwrap().1.trim(),
        format!("drop-shadow(0 3px 3px {red})")
    );
    // None of a filter's functions set: nothing to apply.
    assert_eq!(last(&["filter"]), None);
    assert_eq!(last(&["blur-none"]), None);
}

#[test]
fn masks_intersect_their_gradients() {
    let image = |classes: &[&str]| {
        let all = applied(classes, &[]);
        (all.iter())
            .find(|(p, ..)| p == "mask-image")
            .map(|(_, v, _)| v.clone())
            .unwrap()
    };
    let white = "linear-gradient(#fff, #fff)";
    assert_eq!(
        image(&["mask-b-from-50%", "mask-t-to-75%"]),
        format!(
            "{white}, {white}, linear-gradient(to bottom, black 50%, transparent 100%), \
             linear-gradient(to top, black 0%, transparent 75%), {white}, {white}"
        )
    );
    assert_eq!(
        image(&["mask-radial-from-20%", "mask-circle", "mask-radial-at-top"]),
        format!(
            "{white}, radial-gradient(circle farthest-corner at top, black 20%, transparent \
             100%), {white}"
        )
    );
    assert_eq!(
        image(&["mask-conic-90", "mask-conic-to-50%"]),
        format!(
            "{white}, {white}, conic-gradient(from calc(1deg * 90), black 0%, transparent 50%)"
        )
    );
}

#[test]
fn gradients_read_their_color_stops() {
    let image = |classes: &[&str]| applied(classes, &[]).pop().unwrap().1;
    // Mixed in Oklab, as the engine takes Lab, from 0% to 100% unless the stops say.
    assert_eq!(
        image(&["bg-linear-to-r", "from-[#f00]", "to-[#00f]"]),
        "linear-gradient(to right in oklab, #f00 0%, #00f 100%)"
    );
    assert_eq!(
        image(&[
            "to-[#00f]",
            "via-[#0f0]",
            "via-20%",
            "from-[#f00]",
            "bg-linear-90"
        ]),
        "linear-gradient(90deg in oklab, #f00 0%, #0f0 20%, #00f 100%)"
    );
    // Without stops the gradient is not drawn; one stop goes to transparent.
    assert!(applied(&["bg-radial"], &[]).is_empty());
    assert_eq!(
        image(&["bg-conic", "from-[#f00]"]),
        "conic-gradient(in oklab, #f00 0%, #0000 100%)"
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
