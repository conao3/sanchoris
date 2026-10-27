use std::collections::BTreeMap;

use async_graphql::parser::parse_schema;
use async_graphql::parser::types::{InputValueDefinition, TypeKind, TypeSystemDefinition};

const CONTRACT: &str = include_str!("../../../apps/frontend/src/graphql/schema.graphql");
const BUILT_IN: &[&str] = &["String", "Int", "Float", "Boolean", "ID"];

fn input_value(value: &InputValueDefinition) -> String {
    match &value.default_value {
        Some(default) => format!("{}: {} = {}", value.name.node, value.ty.node, default.node),
        None => format!("{}: {}", value.name.node, value.ty.node),
    }
}

fn normalize(sdl: &str) -> BTreeMap<String, String> {
    let document = parse_schema(sdl).expect("the SDL parses");
    let mut types = BTreeMap::new();
    for definition in document.definitions {
        let TypeSystemDefinition::Type(definition) = definition else {
            continue;
        };
        let definition = definition.node;
        let name = definition.name.node.to_string();
        if BUILT_IN.contains(&name.as_str()) {
            continue;
        }
        let mut members = match &definition.kind {
            TypeKind::Scalar => vec![],
            TypeKind::Object(object) => object
                .fields
                .iter()
                .map(|field| {
                    let field = &field.node;
                    let mut arguments = field
                        .arguments
                        .iter()
                        .map(|argument| input_value(&argument.node))
                        .collect::<Vec<_>>();
                    arguments.sort();
                    format!(
                        "{}({}): {}",
                        field.name.node,
                        arguments.join(", "),
                        field.ty.node
                    )
                })
                .collect(),
            TypeKind::Enum(enumeration) => enumeration
                .values
                .iter()
                .map(|value| value.node.value.node.to_string())
                .collect(),
            TypeKind::InputObject(input) => input
                .fields
                .iter()
                .map(|field| input_value(&field.node))
                .collect(),
            TypeKind::Interface(_) | TypeKind::Union(_) => {
                panic!("the contract has no interfaces or unions: {name}")
            }
        };
        members.sort();
        let kind = match &definition.kind {
            TypeKind::Scalar => "scalar",
            TypeKind::Object(_) => "type",
            TypeKind::Enum(_) => "enum",
            TypeKind::InputObject(_) => "input",
            TypeKind::Interface(_) | TypeKind::Union(_) => unreachable!(),
        };
        types.insert(name, format!("{kind} {{ {} }}", members.join("; ")));
    }
    types
}

#[test]
fn exported_schema_matches_the_frontend_contract() {
    let contract = normalize(CONTRACT);
    let exported = normalize(&sanchoris_graphql::schema_sdl());
    let mut drift = Vec::new();
    for (name, expected) in &contract {
        match exported.get(name) {
            None => drift.push(format!("missing {name}: {expected}")),
            Some(actual) if actual != expected => drift.push(format!(
                "{name}\n  contract: {expected}\n  exported: {actual}"
            )),
            Some(_) => {}
        }
    }
    for name in exported.keys().filter(|name| !contract.contains_key(*name)) {
        drift.push(format!("extra {name}: {}", exported[name]));
    }
    assert!(drift.is_empty(), "schema drift:\n{}", drift.join("\n"));
}
