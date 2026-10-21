//! JSX spread attributes. `{...object}` stands for the attributes the object sets, written in its
//! place, so every attribute consumer sees ordinary attributes.

use super::*;
use oxc::allocator::CloneIn;
use oxc::ast::ast::PropertyKind;
use oxc::ast::builder::AstBuilder;
use oxc::str::{Ident, Str};

/// Prefix of the hidden locals that hold spread values which have no expression in the source.
pub(super) const SPREAD_BINDING: &str = "__valleSpread";

enum SpreadField<'s> {
    /// A property of an object written in the source keeps its expression.
    Expression(&'s Expression<'s>),
    /// A value bound outside the element, such as a prop or a field of prepared data.
    Value(AuthorValue),
}

type Attributes<'s> = Vec<(Option<String>, bool, &'s JSXAttribute<'s>)>;

impl<'s> Compiler<'s> {
    /// The attributes of `element` with each spread written out in its place. As in React, the
    /// last attribute of a name wins, and a spread never sets `key`. Children that a spread
    /// passes apply when the element has none of its own.
    pub(super) fn jsx_attributes(
        &mut self,
        element: &'s JSXElement<'s>,
    ) -> (Vec<&'s JSXAttribute<'s>>, Option<Vec<PendingNode>>) {
        let attributes = &element.opening_element.attributes;
        if !attributes
            .iter()
            .any(|item| matches!(item, JSXAttributeItem::SpreadAttribute(_)))
        {
            let written = attributes.iter().filter_map(|item| match item {
                JSXAttributeItem::Attribute(attribute) => Some(&**attribute),
                JSXAttributeItem::SpreadAttribute(_) => None,
            });
            return (written.collect(), None);
        }
        let allocator = self.allocator;
        let builder = AstBuilder::new(allocator);
        let mut items = Attributes::new();
        let mut children = None;
        for item in attributes {
            let spread = match item {
                JSXAttributeItem::Attribute(attribute) => {
                    let name = match &attribute.name {
                        JSXAttributeName::Identifier(name) => Some(name.name.to_string()),
                        JSXAttributeName::NamespacedName(_) => None,
                    };
                    push_attribute(&mut items, name, false, attribute);
                    continue;
                }
                JSXAttributeItem::SpreadAttribute(spread) => spread,
            };
            let Some(fields) = self.spread_fields(&spread.argument, 0) else {
                continue;
            };
            for (name, field) in fields {
                let value = match field {
                    _ if name == "key" => continue,
                    SpreadField::Value(AuthorValue::Children(nodes)) => {
                        children = Some(nodes);
                        continue;
                    }
                    SpreadField::Expression(expression) => expression.clone_in(allocator),
                    SpreadField::Value(value) => {
                        let hidden = format!("{SPREAD_BINDING}{}", self.spread_bindings);
                        self.spread_bindings += 1;
                        self.bind_author_value(hidden.clone(), value);
                        Expression::new_identifier(
                            spread.span,
                            Ident::from_str_in(&hidden, &builder),
                            &builder,
                        )
                    }
                };
                let attribute = allocator.alloc(JSXAttribute::new(
                    spread.span,
                    JSXAttributeName::new_identifier(
                        spread.span,
                        Str::from_str_in(&name, &builder),
                        &builder,
                    ),
                    Some(JSXAttributeValue::new_expression_container(
                        spread.span,
                        JSXExpression::from(value),
                        &builder,
                    )),
                    &builder,
                ));
                push_attribute(&mut items, Some(name), true, attribute);
            }
        }
        (
            items.into_iter().map(|(_, _, item)| item).collect(),
            children,
        )
    }

    /// The fields a spread object sets, in order.
    fn spread_fields(
        &mut self,
        argument: &'s Expression<'s>,
        depth: usize,
    ) -> Option<Vec<(String, SpreadField<'s>)>> {
        if depth > 64 {
            self.illegal(
                DiagCode::GrammarForbidden,
                argument.span(),
                "spread objects nest more than 64 levels",
            );
            return None;
        }
        if let Some(object) = self.spread_object_literal(argument) {
            let mut fields = Vec::new();
            for property in &object.properties {
                let property = match property {
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        fields.extend(self.spread_fields(&spread.argument, depth + 1)?);
                        continue;
                    }
                    ObjectPropertyKind::ObjectProperty(property) => property,
                };
                let name = if property.method || property.kind != PropertyKind::Init {
                    None
                } else if property.computed {
                    property
                        .key
                        .as_expression()
                        .and_then(|key| self.eval_static(key))
                        .and_then(|key| key.as_str().map(str::to_owned))
                } else {
                    static_property_name(&property.key)
                };
                let Some(name) = name else {
                    self.illegal(
                        DiagCode::GrammarForbidden,
                        property.span(),
                        "a spread object sets attributes with `name: value` properties whose names are prepare-time strings",
                    );
                    return None;
                };
                fields.push((name, SpreadField::Expression(&property.value)));
            }
            return Some(fields);
        }
        let expression = peel_expr(argument);
        if let Expression::Identifier(identifier) = expression
            && let Some(props) = self.spread_props(identifier.name.as_str(), expression.span())
        {
            return Some(
                props
                    .into_iter()
                    .map(|(name, value)| (name, SpreadField::Value(value)))
                    .collect(),
            );
        }
        if let Some(fields) = self.instance_object_fields(expression) {
            return Some(
                fields
                    .into_iter()
                    .map(|(name, (column, value_type))| {
                        let expr = self.push(
                            Expr::InstanceField { column, value_type },
                            expression.span(),
                        );
                        (name, SpreadField::Value(AuthorValue::Dynamic(expr)))
                    })
                    .collect(),
            );
        }
        match self.eval_static(expression) {
            Some(serde_json::Value::Object(object)) => Some(
                object
                    .into_iter()
                    .map(|(name, value)| (name, SpreadField::Value(AuthorValue::Static(value))))
                    .collect(),
            ),
            _ => {
                self.unsupported(
                    argument.span(),
                    "spread an object literal, `props`, a map row or a prepare-time object",
                );
                None
            }
        }
    }

    /// The object literal a spread names directly, through a local object or a module constant.
    fn spread_object_literal(
        &self,
        argument: &'s Expression<'s>,
    ) -> Option<&'s ObjectExpression<'s>> {
        let expression = match peel_expr(argument) {
            Expression::Identifier(identifier) => {
                let name = identifier.name.as_str();
                match self.bindings.objects.get(name) {
                    Some(object) => peel_expr(object.expression()),
                    None if self.is_local_name(name) => return None,
                    None => peel_expr(self.module_const_inits.get(name).copied()?),
                }
            }
            expression => expression,
        };
        match expression {
            Expression::ObjectExpression(object) => Some(object),
            _ => None,
        }
    }

    /// The props a spread of `props`, or of the rest of destructured props, passes on.
    fn spread_props(&mut self, name: &str, span: Span) -> Option<Vec<(String, AuthorValue)>> {
        if self.is_local_name(name) {
            return None;
        }
        let excluded = match self.bindings.prop_rests.get(name) {
            Some(excluded) => excluded.clone(),
            None if name == "props" => BTreeSet::new(),
            None => return None,
        };
        if let Some(props) = &self.bindings.component_props {
            return Some(
                props
                    .iter()
                    .filter(|(prop, _)| !excluded.contains(*prop))
                    .map(|(prop, value)| (prop.clone(), value.clone()))
                    .collect(),
            );
        }
        let names = self
            .controls
            .props
            .keys()
            .filter(|prop| !excluded.contains(*prop))
            .cloned()
            .collect::<Vec<_>>();
        Some(
            names
                .into_iter()
                .map(|prop| {
                    let expr = self.push(Expr::Prop { name: prop.clone() }, span);
                    (prop, AuthorValue::Dynamic(expr))
                })
                .collect(),
        )
    }

    /// Whether `name` is bound in the current component or helper.
    pub(super) fn is_local_name(&self, name: &str) -> bool {
        self.bindings.scalars.contains_key(name)
            || self.bindings.statics.contains_key(name)
            || self.bindings.tuples.contains_key(name)
            || self.bindings.paints.contains_key(name)
            || self.bindings.instance_objects.contains_key(name)
            || self.bindings.children.contains_key(name)
            || self.bindings.objects.contains_key(name)
            || self.bindings.local_functions.contains_key(name)
            || self.bindings.times.contains_key(name)
            || self.bindings.captions.contains_key(name)
    }

    /// Bind a component prop or helper argument under a local name.
    pub(super) fn bind_author_value(&mut self, name: String, value: AuthorValue) {
        match value {
            AuthorValue::Static(value) => self.bind_static(name, value),
            AuthorValue::Dynamic(expr) => self.bind_dynamic(name, expr),
            AuthorValue::Paint(paint) => self.bind_paint(name, paint),
            AuthorValue::DynamicTuple(values) => self.bind_dynamic_tuple(name, values),
            AuthorValue::InstanceObject(fields) => self.bind_instance_object(name, fields),
            AuthorValue::Children(children) => self.bind_children(name, children),
            AuthorValue::Time(path) => self.bind_time(name, path),
            AuthorValue::Caption(path) => self.bind_caption(name, path),
        }
    }
}

/// Add an attribute. One that a spread sets, or that follows a spread that set its name,
/// replaces the earlier attribute of that name; two written attributes of one name stay for
/// their consumer to reject.
fn push_attribute<'s>(
    items: &mut Attributes<'s>,
    name: Option<String>,
    spread: bool,
    item: &'s JSXAttribute<'s>,
) {
    if let Some(name) = &name
        && let Some(at) = items.iter().position(|(other, other_spread, _)| {
            other.as_ref() == Some(name) && (spread || *other_spread)
        })
    {
        items.remove(at);
    }
    items.push((name, spread, item));
}
