use crate::ir::{ApiSpec, FieldDef, RustType, TypeDef};

/// Generate the `src/api/types.rs` file from the API spec.
///
/// Produces serde-compatible structs and enums with proper derive macros,
/// serde annotations, and `#[serde(flatten)] pub extra: serde_json::Value`
/// on response types.
#[must_use]
pub fn generate(spec: &ApiSpec) -> String {
    let mut out = String::with_capacity(8192);

    out.push_str("use serde::{Deserialize, Serialize};\n\n");

    for typedef in &spec.types {
        if typedef.is_enum {
            generate_enum(&mut out, typedef);
        } else {
            generate_struct(&mut out, typedef, spec);
        }
    }

    out
}

fn generate_enum(out: &mut String, typedef: &TypeDef) {
    // Doc comment
    if let Some(ref desc) = typedef.description {
        out.push_str(&format!("/// {desc}\n"));
    }

    out.push_str("#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\n");

    // Determine serde rename strategy from variants
    let rename = infer_enum_rename(typedef);
    if let Some(ref rename_all) = rename {
        out.push_str(&format!("#[serde(rename_all = \"{rename_all}\")]\n"));
    }

    out.push_str(&format!("pub enum {} {{\n", typedef.rust_name));

    for variant in &typedef.enum_variants {
        // If there's no global rename_all that handles it, add explicit rename
        if rename.is_none() && variant.name != variant.rust_name {
            out.push_str(&format!(
                "    #[serde(rename = \"{}\")]\n",
                variant.name
            ));
        }
        out.push_str(&format!("    {},\n", variant.rust_name));
    }

    out.push_str("}\n\n");

    // Display impl
    out.push_str(&format!(
        "impl std::fmt::Display for {} {{\n",
        typedef.rust_name
    ));
    out.push_str("    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n");
    out.push_str("        match self {\n");
    for variant in &typedef.enum_variants {
        out.push_str(&format!(
            "            Self::{} => write!(f, \"{}\"),\n",
            variant.rust_name, variant.name
        ));
    }
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
}

fn generate_struct(out: &mut String, typedef: &TypeDef, spec: &ApiSpec) {
    // Doc comment
    if let Some(ref desc) = typedef.description {
        out.push_str(&format!("/// {desc}\n"));
    }

    out.push_str("#[derive(Debug, Clone, Serialize, Deserialize)]\n");

    // Determine if this API uses camelCase field naming
    if uses_camel_case(typedef) {
        out.push_str("#[serde(rename_all = \"camelCase\")]\n");
    }

    out.push_str(&format!("pub struct {} {{\n", typedef.rust_name));

    for field in &typedef.fields {
        generate_field(out, field);
    }

    // Response types get a serde(flatten) extra field to capture unknown properties
    if is_response_type(&typedef.rust_name, spec) {
        out.push_str("    #[serde(flatten)]\n");
        out.push_str("    pub extra: serde_json::Value,\n");
    }

    out.push_str("}\n\n");
}

fn generate_field(out: &mut String, field: &FieldDef) {
    // Doc comment
    if let Some(ref desc) = field.description {
        out.push_str(&format!("    /// {desc}\n"));
    }

    if !field.required {
        if field.rust_type.is_option() {
            out.push_str(
                "    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n",
            );
        } else {
            out.push_str("    #[serde(default)]\n");
        }
    }

    // If the Rust field name differs from the original, add a rename
    // (but only if camelCase rename_all won't handle it, e.g., reserved keywords)
    if field.rust_name != field.name && !is_camel_to_snake(&field.name, &field.rust_name) {
        out.push_str(&format!("    #[serde(rename = \"{}\")]\n", field.name));
    }

    // Handle reserved keywords
    let field_name = if is_rust_keyword(&field.rust_name) {
        format!("r#{}", field.rust_name)
    } else {
        field.rust_name.clone()
    };

    out.push_str(&format!(
        "    pub {}: {},\n",
        field_name,
        rust_type_to_string(&field.rust_type)
    ));
}

fn rust_type_to_string(rt: &RustType) -> String {
    rt.to_string()
}

/// Check if `name` -> `rust_name` is a standard `camelCase` to `snake_case` conversion.
fn is_camel_to_snake(name: &str, rust_name: &str) -> bool {
    use heck::ToSnakeCase;
    name.to_snake_case() == *rust_name
}

/// Determine if any fields suggest `camelCase` API naming.
fn uses_camel_case(typedef: &TypeDef) -> bool {
    typedef.fields.iter().any(|f| {
        // If the original name contains uppercase (camelCase) but the rust_name is snake_case
        f.name != f.rust_name && f.name.chars().any(|c| c.is_ascii_uppercase())
    })
}

/// Check if a type is used as a response type in any operation.
fn is_response_type(rust_name: &str, spec: &ApiSpec) -> bool {
    spec.operations.iter().any(|op| {
        op.response_type
            .as_ref()
            .is_some_and(|rt| rt.contains_named(rust_name))
    })
}

/// Infer the serde `rename_all` strategy for enum variants.
fn infer_enum_rename(typedef: &TypeDef) -> Option<String> {
    if typedef.enum_variants.is_empty() {
        return None;
    }

    // Check if all values are SCREAMING_SNAKE_CASE
    let all_screaming = typedef
        .enum_variants
        .iter()
        .all(|v| v.name == v.name.to_ascii_uppercase() && v.name.contains('_'));
    if all_screaming {
        return Some("SCREAMING_SNAKE_CASE".into());
    }

    // Check if all values are snake_case
    let all_snake = typedef
        .enum_variants
        .iter()
        .all(|v| v.name == v.name.to_ascii_lowercase() && v.name.contains('_'));
    if all_snake {
        return Some("snake_case".into());
    }

    // Check if all values are lowercase (no separators)
    let all_lower = typedef
        .enum_variants
        .iter()
        .all(|v| v.name == v.name.to_ascii_lowercase());
    if all_lower {
        return Some("lowercase".into());
    }

    // Check if all values are UPPERCASE (no separators)
    let all_upper = typedef
        .enum_variants
        .iter()
        .all(|v| v.name == v.name.to_ascii_uppercase() && !v.name.contains('_'));
    if all_upper {
        return Some("UPPERCASE".into());
    }

    None
}

fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#gen::testutil::{make_enum, make_field, make_spec_with as make_spec, make_struct};
    use crate::ir::{EnumVariant, FieldDef, HttpMethod, Operation, RustType, TypeDef};

    #[test]
    fn generate_empty_spec_produces_import() {
        let spec = make_spec(vec![], vec![]);
        let code = generate(&spec);
        assert!(code.contains("use serde::{Deserialize, Serialize};"));
    }

    #[test]
    fn generate_struct_with_required_fields() {
        let pet = make_struct(
            "Pet",
            vec![
                make_field("id", RustType::I64, true),
                make_field("name", RustType::String, true),
            ],
        );
        let spec = make_spec(vec![pet], vec![]);
        let code = generate(&spec);

        assert!(code.contains("pub struct Pet {"));
        assert!(code.contains("pub id: i64,"));
        assert!(code.contains("pub name: String,"));
    }

    #[test]
    fn generate_struct_with_optional_field() {
        let pet = make_struct(
            "Pet",
            vec![make_field("tag", RustType::Option(Box::new(RustType::String)), false)],
        );
        let spec = make_spec(vec![pet], vec![]);
        let code = generate(&spec);

        assert!(code.contains("pub tag: Option<String>,"));
        assert!(
            code.contains("skip_serializing_if"),
            "optional fields should have skip_serializing_if"
        );
    }

    #[test]
    fn generate_struct_derive_macros() {
        let item = make_struct("Item", vec![]);
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[derive(Debug, Clone, Serialize, Deserialize)]"));
    }

    #[test]
    fn generate_enum_with_variants() {
        let status = make_enum("PetStatus", vec!["available", "pending", "sold"]);
        let spec = make_spec(vec![status], vec![]);
        let code = generate(&spec);

        assert!(code.contains("pub enum PetStatus {"));
        assert!(code.contains("Available,"));
        assert!(code.contains("Pending,"));
        assert!(code.contains("Sold,"));
        assert!(
            code.contains("#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]"),
            "enums should derive PartialEq + Eq"
        );
    }

    #[test]
    fn generate_enum_display_impl() {
        let status = make_enum("Status", vec!["active", "inactive"]);
        let spec = make_spec(vec![status], vec![]);
        let code = generate(&spec);

        assert!(code.contains("impl std::fmt::Display for Status {"));
        assert!(code.contains("Self::Active => write!(f, \"active\")"));
        assert!(code.contains("Self::Inactive => write!(f, \"inactive\")"));
    }

    #[test]
    fn generate_enum_screaming_snake_case_rename() {
        let mode = TypeDef {
            name: "Mode".into(),
            rust_name: "Mode".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![
                EnumVariant {
                    name: "READ_ONLY".into(),
                    rust_name: "ReadOnly".into(),
                },
                EnumVariant {
                    name: "READ_WRITE".into(),
                    rust_name: "ReadWrite".into(),
                },
            ],
            description: None,
        };
        let spec = make_spec(vec![mode], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("SCREAMING_SNAKE_CASE"),
            "all-uppercase+underscore variants should get SCREAMING_SNAKE_CASE rename"
        );
    }

    #[test]
    fn generate_enum_lowercase_rename() {
        let prio = TypeDef {
            name: "Priority".into(),
            rust_name: "Priority".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![
                EnumVariant {
                    name: "low".into(),
                    rust_name: "Low".into(),
                },
                EnumVariant {
                    name: "high".into(),
                    rust_name: "High".into(),
                },
            ],
            description: None,
        };
        let spec = make_spec(vec![prio], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("lowercase"),
            "all-lowercase variants should get lowercase rename"
        );
    }

    #[test]
    fn generate_struct_with_doc_comment() {
        let mut item = make_struct("Item", vec![]);
        item.description = Some("A store item.".into());
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(code.contains("/// A store item."));
    }

    #[test]
    fn generate_field_with_doc_comment() {
        let mut field = make_field("name", RustType::String, true);
        field.description = Some("The item name.".into());
        let item = make_struct("Item", vec![field]);
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(code.contains("/// The item name."));
    }

    #[test]
    fn generate_response_type_gets_flatten_extra() {
        let pet = make_struct(
            "Pet",
            vec![make_field("id", RustType::I64, true)],
        );
        let op = Operation {
            id: "get_pet".into(),
            method: HttpMethod::Get,
            path: "/pets/{id}".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Named("Pet".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![pet], vec![op]);
        let code = generate(&spec);

        assert!(
            code.contains("#[serde(flatten)]"),
            "response types should get flatten extra field"
        );
        assert!(code.contains("pub extra: serde_json::Value,"));
    }

    #[test]
    fn non_response_type_no_flatten() {
        let req = make_struct(
            "CreatePetRequest",
            vec![make_field("name", RustType::String, true)],
        );
        let spec = make_spec(vec![req], vec![]);
        let code = generate(&spec);
        assert!(
            !code.contains("flatten"),
            "non-response types should not have flatten"
        );
    }

    #[test]
    fn generate_vec_field() {
        let item = make_struct(
            "ListResp",
            vec![make_field(
                "items",
                RustType::Vec(Box::new(RustType::String)),
                true,
            )],
        );
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub items: Vec<String>,"));
    }

    #[test]
    fn camel_case_fields_get_rename_all() {
        let item = TypeDef {
            name: "Item".into(),
            rust_name: "Item".into(),
            fields: vec![FieldDef {
                name: "firstName".into(),
                rust_name: "first_name".into(),
                rust_type: RustType::String,
                required: true,
                description: None,
                default_value: None,
            }],
            is_enum: false,
            enum_variants: vec![],
            description: None,
        };
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("camelCase"),
            "fields with camelCase names should trigger rename_all"
        );
    }

    #[test]
    fn rust_keyword_field_is_escaped() {
        let item = make_struct("Item", vec![]);
        let mut item_with_keyword = item;
        item_with_keyword.fields.push(FieldDef {
            name: "type".into(),
            rust_name: "type".into(),
            rust_type: RustType::String,
            required: true,
            description: None,
            default_value: None,
        });
        let spec = make_spec(vec![item_with_keyword], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("pub r#type: String,"),
            "Rust keywords should be escaped with r#"
        );
    }

    #[test]
    fn is_rust_keyword_detects_common_keywords() {
        assert!(is_rust_keyword("type"));
        assert!(is_rust_keyword("match"));
        assert!(is_rust_keyword("async"));
        assert!(is_rust_keyword("self"));
        assert!(!is_rust_keyword("name"));
        assert!(!is_rust_keyword("id"));
    }

    #[test]
    fn rust_type_to_string_all_variants() {
        assert_eq!(rust_type_to_string(&RustType::String), "String");
        assert_eq!(rust_type_to_string(&RustType::I64), "i64");
        assert_eq!(rust_type_to_string(&RustType::U64), "u64");
        assert_eq!(rust_type_to_string(&RustType::F64), "f64");
        assert_eq!(rust_type_to_string(&RustType::Bool), "bool");
        assert_eq!(rust_type_to_string(&RustType::Value), "serde_json::Value");
        assert_eq!(
            rust_type_to_string(&RustType::Vec(Box::new(RustType::I64))),
            "Vec<i64>"
        );
        assert_eq!(
            rust_type_to_string(&RustType::Option(Box::new(RustType::Bool))),
            "Option<bool>"
        );
        assert_eq!(
            rust_type_to_string(&RustType::Named("Foo".into())),
            "Foo"
        );
    }

    #[test]
    fn is_camel_to_snake_correct() {
        assert!(is_camel_to_snake("firstName", "first_name"));
        assert!(is_camel_to_snake("id", "id"));
        assert!(!is_camel_to_snake("some_field", "somefield"));
    }

    #[test]
    fn infer_enum_rename_none_for_mixed_case() {
        let mixed = TypeDef {
            name: "Mixed".into(),
            rust_name: "Mixed".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![
                EnumVariant {
                    name: "Active".into(),
                    rust_name: "Active".into(),
                },
                EnumVariant {
                    name: "inactive".into(),
                    rust_name: "Inactive".into(),
                },
            ],
            description: None,
        };
        assert!(infer_enum_rename(&mixed).is_none());
    }

    #[test]
    fn infer_enum_rename_empty_variants() {
        let empty = TypeDef {
            name: "Empty".into(),
            rust_name: "Empty".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![],
            description: None,
        };
        assert!(infer_enum_rename(&empty).is_none());
    }

    // -- UPPERCASE enum rename --

    #[test]
    fn infer_enum_rename_uppercase() {
        let mode = TypeDef {
            name: "Mode".into(),
            rust_name: "Mode".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![
                EnumVariant {
                    name: "GET".into(),
                    rust_name: "Get".into(),
                },
                EnumVariant {
                    name: "POST".into(),
                    rust_name: "Post".into(),
                },
            ],
            description: None,
        };
        assert_eq!(infer_enum_rename(&mode), Some("UPPERCASE".into()));
    }

    // -- snake_case enum rename --

    #[test]
    fn infer_enum_rename_snake_case() {
        let mode = TypeDef {
            name: "Mode".into(),
            rust_name: "Mode".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![
                EnumVariant {
                    name: "read_only".into(),
                    rust_name: "ReadOnly".into(),
                },
                EnumVariant {
                    name: "read_write".into(),
                    rust_name: "ReadWrite".into(),
                },
            ],
            description: None,
        };
        assert_eq!(infer_enum_rename(&mode), Some("snake_case".into()));
    }

    // -- Non-required Vec field gets #[serde(default)] --

    #[test]
    fn non_required_vec_field_gets_default() {
        let item = make_struct(
            "Item",
            vec![make_field(
                "tags",
                RustType::Vec(Box::new(RustType::String)),
                false,
            )],
        );
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("#[serde(default)]"),
            "non-required Vec field should get #[serde(default)]"
        );
    }

    // -- Non-required non-Option non-Vec gets #[serde(default)] --

    #[test]
    fn non_required_plain_field_gets_default() {
        let mut field = make_field("count", RustType::I64, false);
        field.rust_type = RustType::I64;
        let item = make_struct("Item", vec![field]);
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("#[serde(default)]"),
            "non-required plain field should get #[serde(default)]"
        );
    }

    // -- Explicit serde rename when field name is not camel-to-snake --

    #[test]
    fn explicit_rename_for_non_camel_field() {
        let item = TypeDef {
            name: "Item".into(),
            rust_name: "Item".into(),
            fields: vec![FieldDef {
                name: "custom_name".into(),
                rust_name: "different_name".into(),
                rust_type: RustType::String,
                required: true,
                description: None,
                default_value: None,
            }],
            is_enum: false,
            enum_variants: vec![],
            description: None,
        };
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("#[serde(rename = \"custom_name\")]"),
            "field whose rust_name doesn't match name via camel->snake should get explicit rename"
        );
    }

    // -- Response type inside Vec still gets flatten extra --

    #[test]
    fn response_type_inside_vec_gets_flatten() {
        let pet = make_struct(
            "Pet",
            vec![make_field("id", RustType::I64, true)],
        );
        let op = Operation {
            id: "list_pets".into(),
            method: HttpMethod::Get,
            path: "/pets".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Vec(Box::new(RustType::Named("Pet".into())))),
            errors: vec![],
        };
        let spec = make_spec(vec![pet], vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("#[serde(flatten)]"),
            "type used in Vec<Named> response should get flatten"
        );
    }

    // -- Enum variant with explicit rename when no rename_all --

    #[test]
    fn enum_variant_explicit_rename_when_mixed() {
        let mixed = TypeDef {
            name: "Mixed".into(),
            rust_name: "Mixed".into(),
            fields: Vec::new(),
            is_enum: true,
            enum_variants: vec![
                EnumVariant {
                    name: "Active".into(),
                    rust_name: "Active".into(),
                },
                EnumVariant {
                    name: "in-progress".into(),
                    rust_name: "InProgress".into(),
                },
            ],
            description: None,
        };
        let spec = make_spec(vec![mixed], vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("#[serde(rename = \"in-progress\")]"),
            "variant whose name differs from rust_name with no rename_all should get explicit rename"
        );
        assert!(
            !code.contains("#[serde(rename = \"Active\")]"),
            "variant where name == rust_name should not get rename"
        );
    }

    // -- Enum with description doc comment --

    #[test]
    fn enum_with_description_has_doc() {
        let mut status = make_enum("Status", vec!["active", "inactive"]);
        status.description = Some("Current status of the resource.".into());
        let spec = make_spec(vec![status], vec![]);
        let code = generate(&spec);
        assert!(code.contains("/// Current status of the resource."));
    }

    // -- Multiple structs are generated in order --

    #[test]
    fn multiple_types_all_generated() {
        let t1 = make_struct("Alpha", vec![make_field("a", RustType::String, true)]);
        let t2 = make_struct("Beta", vec![make_field("b", RustType::I64, true)]);
        let t3 = make_enum("Gamma", vec!["x", "y"]);
        let spec = make_spec(vec![t1, t2, t3], vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub struct Alpha {"));
        assert!(code.contains("pub struct Beta {"));
        assert!(code.contains("pub enum Gamma {"));
    }

    // -- Named type field --

    #[test]
    fn named_type_field_renders_correctly() {
        let item = make_struct(
            "Item",
            vec![make_field("status", RustType::Named("Status".into()), true)],
        );
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub status: Status,"));
    }

    // -- Value type field --

    #[test]
    fn value_type_field_renders_correctly() {
        let item = make_struct(
            "Item",
            vec![make_field("extra", RustType::Value, true)],
        );
        let spec = make_spec(vec![item], vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub extra: serde_json::Value,"));
    }

    // -- All keywords are detected --

    #[test]
    fn is_rust_keyword_async_await() {
        assert!(is_rust_keyword("async"));
        assert!(is_rust_keyword("await"));
        assert!(is_rust_keyword("dyn"));
    }

    // -- uses_camel_case returns false for all-lowercase --

    #[test]
    fn uses_camel_case_false_for_flat() {
        let item = make_struct(
            "Item",
            vec![
                make_field("id", RustType::I64, true),
                make_field("name", RustType::String, true),
            ],
        );
        assert!(!uses_camel_case(&item));
    }

    // -- RustType::contains_named deep nesting --

    #[test]
    fn rust_type_contains_named_in_option_vec() {
        let rt = RustType::Option(Box::new(RustType::Vec(Box::new(RustType::Named(
            "Pet".into(),
        )))));
        assert!(rt.contains_named("Pet"));
        assert!(!rt.contains_named("Dog"));
    }

    #[test]
    fn rust_type_contains_named_primitives_false() {
        assert!(!RustType::String.contains_named("Foo"));
        assert!(!RustType::I64.contains_named("Foo"));
        assert!(!RustType::Value.contains_named("Foo"));
    }
}
