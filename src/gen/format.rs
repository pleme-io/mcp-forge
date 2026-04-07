use crate::ir::{ApiSpec, FieldDef, HttpMethod, Operation, RustType, TypeDef};
use heck::ToSnakeCase;

/// Generate the `src/format.rs` file from the API spec.
///
/// Produces a `format_*` function for each operation's response type.
/// Uses `writeln!` pattern for text output and handles Option fields with if-let.
pub fn generate(spec: &ApiSpec) -> String {
    let mut out = String::with_capacity(8192);

    out.push_str("use crate::api::types::*;\n");
    out.push_str("use std::fmt::Write;\n\n");

    // Truncate helper (always useful)
    out.push_str(
        "pub fn truncate(s: &str, max: usize) -> String {\n\
         \x20   if s.len() > max {\n\
         \x20       format!(\"{}...\", &s[..max.saturating_sub(3)])\n\
         \x20   } else {\n\
         \x20       s.to_string()\n\
         \x20   }\n\
         }\n\n",
    );

    // Generate a format function for each operation that returns a rich response.
    // Skip simple action responses (stop/delete) -- those are handled inline in mcp.rs.
    let mut generated_types = std::collections::HashSet::new();

    for op in &spec.operations {
        // Skip simple action operations
        let is_simple = matches!(op.method, HttpMethod::Delete)
            || op.id.to_snake_case().starts_with("stop")
            || op.id.to_snake_case().starts_with("delete");
        if is_simple {
            continue;
        }

        // Skip operations with no response type
        let response_type = match &op.response_type {
            Some(rt) => rt,
            None => continue,
        };

        let response_type_str = rust_type_to_string(response_type);

        // Avoid generating duplicate format functions for the same response type
        if !generated_types.insert(response_type_str.clone()) {
            // Already generated -- emit a thin alias
            let fn_name = format!("format_{}", op.id.to_snake_case());
            let existing_fn = find_format_fn_for_type(response_type, &spec.operations, op);
            if let Some(ref existing) = existing_fn {
                out.push_str(&format!(
                    "pub fn {fn_name}(data: &{response_type_str}) -> String {{\n\
                     \x20   {existing}(data)\n\
                     }}\n\n"
                ));
            }
            continue;
        }

        let fn_name = format!("format_{}", op.id.to_snake_case());

        // Find the TypeDef for this response type (only for Named types)
        if let RustType::Named(type_name) = response_type {
            if let Some(typedef) = spec.types.iter().find(|t| t.rust_name == *type_name) {
                generate_format_fn(&mut out, &fn_name, &response_type_str, typedef, spec);
            } else {
                generate_generic_format_fn(&mut out, &fn_name, &response_type_str);
            }
        } else {
            generate_generic_format_fn(&mut out, &fn_name, &response_type_str);
        }
    }

    out
}

fn generate_format_fn(
    out: &mut String,
    fn_name: &str,
    type_name: &str,
    typedef: &TypeDef,
    spec: &ApiSpec,
) {
    out.push_str(&format!(
        "pub fn {fn_name}(data: &{type_name}) -> String {{\n"
    ));

    // Check if this is a list type (has a Vec field as its primary content)
    let list_field = typedef.fields.iter().find(|f| is_vec_type(&f.rust_type));

    if let Some(list_field) = list_field {
        // This is a list response -- generate list formatting
        generate_list_format(out, typedef, list_field, spec);
    } else {
        // Single-object response -- format each field
        generate_single_format(out, typedef);
    }

    out.push_str("}\n\n");
}

fn generate_list_format(
    out: &mut String,
    typedef: &TypeDef,
    list_field: &FieldDef,
    spec: &ApiSpec,
) {
    let field_name = &list_field.rust_name;

    out.push_str(&format!(
        "    if data.{field_name}.is_empty() {{\n\
         \x20       return \"No results found.\".into();\n\
         \x20   }}\n"
    ));
    out.push_str(&format!(
        "    let mut out = format!(\"{{}} results:\\n\", data.{field_name}.len());\n"
    ));

    // Get the inner type of the Vec to format individual items
    if let RustType::Vec(inner) = &list_field.rust_type {
        if let RustType::Named(inner_type) = inner.as_ref() {
            // Find the TypeDef for the inner type
            if let Some(inner_typedef) = spec.types.iter().find(|t| t.rust_name == *inner_type) {
                // Format each item compactly
                out.push_str(&format!(
                    "    for item in &data.{field_name} {{\n"
                ));
                generate_compact_item_format(out, inner_typedef);
                out.push_str("    }\n");
            } else {
                // Unknown inner type -- use Debug
                out.push_str(&format!(
                    "    for item in &data.{field_name} {{\n\
                     \x20       let _ = writeln!(out, \"  {{:?}}\", item);\n\
                     \x20   }}\n"
                ));
            }
        } else {
            // Simple type Vec (e.g., Vec<String>)
            out.push_str(&format!(
                "    for item in &data.{field_name} {{\n\
                 \x20       let _ = writeln!(out, \"  {{}}\", item);\n\
                 \x20   }}\n"
            ));
        }
    }

    // Check for pagination cursor field
    let cursor_field = typedef
        .fields
        .iter()
        .find(|f| f.rust_name.contains("cursor") || f.rust_name.contains("next"));
    if let Some(cursor_field) = cursor_field {
        if is_option_type(&cursor_field.rust_type) {
            out.push_str(&format!(
                "    if let Some(ref cursor) = data.{} {{\n\
                 \x20       let _ = writeln!(out, \"\\n[next page: cursor={{}}\", cursor);\n\
                 \x20   }}\n",
                cursor_field.rust_name
            ));
        }
    }

    out.push_str("    out\n");
}

fn generate_compact_item_format(out: &mut String, typedef: &TypeDef) {
    // Build a compact one-line format: "  field1 | field2 | field3"
    let key_fields: Vec<&FieldDef> = typedef
        .fields
        .iter()
        .filter(|f| is_display_field(f))
        .take(4)
        .collect();

    if key_fields.is_empty() {
        out.push_str("        let _ = writeln!(out, \"  {:?}\", item);\n");
        return;
    }

    // Build format parts
    let mut parts = Vec::new();
    let mut args = Vec::new();

    for field in &key_fields {
        if is_option_type(&field.rust_type) {
            parts.push("{}".to_string());
            args.push(format!(
                "item.{}.as_deref().unwrap_or(\"-\")",
                field.rust_name
            ));
        } else {
            parts.push("{}".to_string());
            args.push(format!("item.{}", field.rust_name));
        }
    }

    let format_str = format!("  {}", parts.join(" | "));
    let args_str = args.join(", ");
    out.push_str(&format!(
        "        let _ = writeln!(out, \"{format_str}\", {args_str});\n"
    ));
}

fn generate_single_format(out: &mut String, typedef: &TypeDef) {
    out.push_str("    let mut out = String::with_capacity(512);\n");

    for field in &typedef.fields {
        let label = field_label(&field.rust_name);

        if is_option_type(&field.rust_type) {
            out.push_str(&format!(
                "    if let Some(ref v) = data.{} {{\n\
                 \x20       let _ = writeln!(out, \"{label}: {{v}}\");\n\
                 \x20   }}\n",
                field.rust_name
            ));
        } else if is_vec_type(&field.rust_type) {
            out.push_str(&format!(
                "    if !data.{}.is_empty() {{\n\
                 \x20       let _ = writeln!(out, \"{label}: {{}} items\", data.{}.len());\n\
                 \x20   }}\n",
                field.rust_name, field.rust_name
            ));
        } else {
            out.push_str(&format!(
                "    let _ = writeln!(out, \"{label}: {{}}\", data.{});\n",
                field.rust_name
            ));
        }
    }

    out.push_str("    out\n");
}

fn generate_generic_format_fn(out: &mut String, fn_name: &str, type_name: &str) {
    out.push_str(&format!(
        "pub fn {fn_name}(data: &{type_name}) -> String {{\n\
         \x20   format!(\"{{:#?}}\", data)\n\
         }}\n\n"
    ));
}

/// Find an existing format function name for a given response type.
fn find_format_fn_for_type(
    rust_type: &RustType,
    operations: &[Operation],
    current_op: &Operation,
) -> Option<String> {
    operations
        .iter()
        .filter(|op| {
            op.id != current_op.id
                && op.response_type.as_ref() == Some(rust_type)
        })
        .map(|op| format!("format_{}", op.id.to_snake_case()))
        .next()
}

fn rust_type_to_string(rt: &RustType) -> String {
    match rt {
        RustType::String => "String".into(),
        RustType::I64 => "i64".into(),
        RustType::U64 => "u64".into(),
        RustType::F64 => "f64".into(),
        RustType::Bool => "bool".into(),
        RustType::Vec(inner) => format!("Vec<{}>", rust_type_to_string(inner)),
        RustType::Option(inner) => format!("Option<{}>", rust_type_to_string(inner)),
        RustType::Named(name) => name.clone(),
        RustType::Value => "serde_json::Value".into(),
    }
}

/// Build a label from a field name: "field_name" -> "Field Name"
fn field_label(name: &str) -> String {
    name.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{upper}{}", chars.as_str())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Check if a field should be included in compact display.
fn is_display_field(field: &FieldDef) -> bool {
    // Include simple scalar fields, exclude nested objects and large collections
    matches!(
        &field.rust_type,
        RustType::String
            | RustType::I64
            | RustType::U64
            | RustType::F64
            | RustType::Bool
            | RustType::Option(_)
            | RustType::Named(_)
    )
}

fn is_option_type(rt: &RustType) -> bool {
    matches!(rt, RustType::Option(_))
}

fn is_vec_type(rt: &RustType) -> bool {
    matches!(rt, RustType::Vec(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ApiSpec, AuthMethod, FieldDef, HttpMethod};

    fn make_field(name: &str, rust_type: RustType, required: bool) -> FieldDef {
        FieldDef {
            name: name.into(),
            rust_name: heck::ToSnakeCase::to_snake_case(name),
            rust_type,
            required,
            description: None,
            default_value: None,
        }
    }

    fn make_struct(name: &str, fields: Vec<FieldDef>) -> TypeDef {
        TypeDef {
            name: name.into(),
            rust_name: heck::ToUpperCamelCase::to_upper_camel_case(name),
            fields,
            is_enum: false,
            enum_variants: Vec::new(),
            description: None,
        }
    }

    fn make_get_op_with_response(id: &str, response_type: RustType) -> Operation {
        Operation {
            id: id.into(),
            method: HttpMethod::Get,
            path: format!("/{id}"),
            summary: Some(format!("Get {id}")),
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(response_type),
            errors: vec![],
        }
    }

    fn make_spec(types: Vec<TypeDef>, operations: Vec<Operation>) -> ApiSpec {
        ApiSpec {
            name: "TestApi".into(),
            description: None,
            version: "1.0.0".into(),
            base_url: None,
            auth: AuthMethod::None,
            operations,
            types,
        }
    }

    // -- Top-level generate --

    #[test]
    fn generates_imports() {
        let spec = make_spec(vec![], vec![]);
        let code = generate(&spec);
        assert!(code.contains("use crate::api::types::*;"));
        assert!(code.contains("use std::fmt::Write;"));
    }

    #[test]
    fn generates_truncate_helper() {
        let spec = make_spec(vec![], vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub fn truncate(s: &str, max: usize) -> String"));
    }

    #[test]
    fn generates_format_fn_for_get_operation() {
        let item = make_struct(
            "Item",
            vec![
                make_field("id", RustType::I64, true),
                make_field("name", RustType::String, true),
            ],
        );
        let op = make_get_op_with_response("list_items", RustType::Named("Item".into()));
        let spec = make_spec(vec![item], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub fn format_list_items(data: &Item) -> String"));
    }

    #[test]
    fn skips_delete_operations() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![], vec![op]);
        let code = generate(&spec);
        assert!(!code.contains("format_delete_item"));
    }

    #[test]
    fn skips_operations_with_no_response_type() {
        let op = Operation {
            id: "do_thing".into(),
            method: HttpMethod::Post,
            path: "/thing".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec(vec![], vec![op]);
        let code = generate(&spec);
        assert!(!code.contains("format_do_thing"));
    }

    #[test]
    fn generates_list_format_for_vec_field() {
        let pet = make_struct(
            "Pet",
            vec![
                make_field("id", RustType::I64, true),
                make_field("name", RustType::String, true),
            ],
        );
        let list_resp = make_struct(
            "PetList",
            vec![make_field(
                "pets",
                RustType::Vec(Box::new(RustType::Named("Pet".into()))),
                true,
            )],
        );
        let op = make_get_op_with_response("list_pets", RustType::Named("PetList".into()));
        let spec = make_spec(vec![pet, list_resp], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub fn format_list_pets(data: &PetList) -> String"));
        assert!(code.contains("No results found."));
        assert!(code.contains("data.pets.len()"));
        assert!(code.contains("for item in &data.pets"));
    }

    #[test]
    fn generates_single_format_for_non_list() {
        let item = make_struct(
            "Item",
            vec![
                make_field("id", RustType::I64, true),
                make_field("name", RustType::String, true),
                make_field("tag", RustType::Option(Box::new(RustType::String)), false),
            ],
        );
        let op = make_get_op_with_response("get_item", RustType::Named("Item".into()));
        let spec = make_spec(vec![item], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub fn format_get_item(data: &Item) -> String"));
        // Required fields use direct writeln
        assert!(code.contains("data.id"));
        assert!(code.contains("data.name"));
        // Optional field uses if let
        assert!(code.contains("if let Some(ref v) = data.tag"));
    }

    #[test]
    fn generates_generic_format_for_unknown_type() {
        let op =
            make_get_op_with_response("get_unknown", RustType::Named("UnknownType".into()));
        let spec = make_spec(vec![], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub fn format_get_unknown(data: &UnknownType) -> String"));
        assert!(code.contains("{:#?}"));
    }

    #[test]
    fn generates_generic_format_for_non_named_type() {
        let op = make_get_op_with_response("get_raw", RustType::Value);
        let spec = make_spec(vec![], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub fn format_get_raw(data: &serde_json::Value) -> String"));
        assert!(code.contains("{:#?}"));
    }

    #[test]
    fn duplicate_response_type_gets_alias() {
        let item = make_struct(
            "Item",
            vec![make_field("id", RustType::I64, true)],
        );
        let op1 = make_get_op_with_response("get_item_v1", RustType::Named("Item".into()));
        let op2 = make_get_op_with_response("get_item_v2", RustType::Named("Item".into()));
        let spec = make_spec(vec![item], vec![op1, op2]);
        let code = generate(&spec);
        // First one gets full format fn
        assert!(code.contains("pub fn format_get_item_v1(data: &Item) -> String"));
        // Second one should delegate to first
        assert!(code.contains("pub fn format_get_item_v2(data: &Item) -> String"));
        assert!(code.contains("format_get_item_v1(data)"));
    }

    #[test]
    fn cursor_field_in_list_response() {
        let item = make_struct("Item", vec![make_field("id", RustType::I64, true)]);
        let list_resp = make_struct(
            "ItemList",
            vec![
                make_field(
                    "items",
                    RustType::Vec(Box::new(RustType::Named("Item".into()))),
                    true,
                ),
                make_field("cursor", RustType::Option(Box::new(RustType::String)), false),
            ],
        );
        let op = make_get_op_with_response("list_items", RustType::Named("ItemList".into()));
        let spec = make_spec(vec![item, list_resp], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("cursor"));
        assert!(code.contains("next page"));
    }

    // -- Helper function tests --

    #[test]
    fn field_label_converts_snake_to_title() {
        assert_eq!(field_label("first_name"), "First Name");
        assert_eq!(field_label("id"), "Id");
        assert_eq!(field_label("api_key_file"), "Api Key File");
    }

    #[test]
    fn field_label_empty_string() {
        assert_eq!(field_label(""), "");
    }

    #[test]
    fn is_display_field_scalars() {
        assert!(is_display_field(&make_field("id", RustType::I64, true)));
        assert!(is_display_field(&make_field("name", RustType::String, true)));
        assert!(is_display_field(&make_field("active", RustType::Bool, true)));
        assert!(is_display_field(&make_field(
            "tag",
            RustType::Option(Box::new(RustType::String)),
            false
        )));
    }

    #[test]
    fn is_display_field_rejects_vec() {
        assert!(!is_display_field(&make_field(
            "items",
            RustType::Vec(Box::new(RustType::String)),
            true
        )));
    }

    #[test]
    fn is_display_field_rejects_value() {
        assert!(!is_display_field(&make_field("data", RustType::Value, true)));
    }

    #[test]
    fn is_option_type_tests() {
        assert!(is_option_type(&RustType::Option(Box::new(RustType::String))));
        assert!(!is_option_type(&RustType::String));
    }

    #[test]
    fn is_vec_type_tests() {
        assert!(is_vec_type(&RustType::Vec(Box::new(RustType::I64))));
        assert!(!is_vec_type(&RustType::String));
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
            rust_type_to_string(&RustType::Vec(Box::new(RustType::String))),
            "Vec<String>"
        );
        assert_eq!(
            rust_type_to_string(&RustType::Option(Box::new(RustType::I64))),
            "Option<i64>"
        );
        assert_eq!(
            rust_type_to_string(&RustType::Named("Foo".into())),
            "Foo"
        );
    }

    #[test]
    fn find_format_fn_for_type_finds_existing() {
        let rt = RustType::Named("Item".into());
        let op1 = make_get_op_with_response("get_item", rt.clone());
        let op2 = make_get_op_with_response("fetch_item", rt.clone());
        let ops = vec![op1, op2.clone()];
        let result = find_format_fn_for_type(&rt, &ops, &op2);
        assert_eq!(result, Some("format_get_item".into()));
    }

    #[test]
    fn find_format_fn_for_type_none_when_no_match() {
        let rt = RustType::Named("Item".into());
        let op = make_get_op_with_response("get_item", rt.clone());
        let ops = vec![op.clone()];
        // Only one operation with that type, and it's the current one
        let result = find_format_fn_for_type(&rt, &ops, &op);
        assert!(result.is_none());
    }

    #[test]
    fn generates_vec_field_length_in_single_format() {
        let item = make_struct(
            "Stats",
            vec![
                make_field("count", RustType::I64, true),
                make_field(
                    "tags",
                    RustType::Vec(Box::new(RustType::String)),
                    true,
                ),
            ],
        );
        let op = make_get_op_with_response("get_stats", RustType::Named("Stats".into()));
        let spec = make_spec(vec![item], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("data.tags.len()"));
    }

    // -- stop_ prefix operations are skipped --

    #[test]
    fn skips_stop_operations() {
        let op = Operation {
            id: "stop_service".into(),
            method: HttpMethod::Post,
            path: "/services/{id}/stop".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![], vec![op]);
        let code = generate(&spec);
        assert!(!code.contains("format_stop_service"));
    }

    // -- delete_ prefix operations are also skipped --

    #[test]
    fn skips_delete_prefixed_operations() {
        let op = Operation {
            id: "delete_cache".into(),
            method: HttpMethod::Post,
            path: "/cache/clear".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![], vec![op]);
        let code = generate(&spec);
        assert!(!code.contains("format_delete_cache"));
    }

    // -- Simple Vec<String> list format --

    #[test]
    fn generates_simple_vec_list_format() {
        let list_resp = make_struct(
            "NameList",
            vec![make_field(
                "names",
                RustType::Vec(Box::new(RustType::String)),
                true,
            )],
        );
        let op = make_get_op_with_response("list_names", RustType::Named("NameList".into()));
        let spec = make_spec(vec![list_resp], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("for item in &data.names"));
        assert!(code.contains("writeln!(out, \"  {}\", item)"));
    }

    // -- Compact format with no display fields uses Debug --

    #[test]
    fn compact_format_no_display_fields_uses_debug() {
        let inner = make_struct(
            "Blob",
            vec![
                make_field("data", RustType::Value, true),
                make_field(
                    "nested",
                    RustType::Vec(Box::new(RustType::String)),
                    true,
                ),
            ],
        );
        let list_resp = make_struct(
            "BlobList",
            vec![make_field(
                "blobs",
                RustType::Vec(Box::new(RustType::Named("Blob".into()))),
                true,
            )],
        );
        let op = make_get_op_with_response("list_blobs", RustType::Named("BlobList".into()));
        let spec = make_spec(vec![inner, list_resp], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("{:?}"), "should use Debug for items with no display fields");
    }

    // -- Option fields in compact item format use as_deref --

    #[test]
    fn compact_format_option_field_uses_as_deref() {
        let inner = make_struct(
            "Widget",
            vec![
                make_field("id", RustType::I64, true),
                make_field("label", RustType::Option(Box::new(RustType::String)), false),
            ],
        );
        let list_resp = make_struct(
            "WidgetList",
            vec![make_field(
                "widgets",
                RustType::Vec(Box::new(RustType::Named("Widget".into()))),
                true,
            )],
        );
        let op = make_get_op_with_response("list_widgets", RustType::Named("WidgetList".into()));
        let spec = make_spec(vec![inner, list_resp], vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("as_deref().unwrap_or(\"-\")"),
            "option fields in compact display should use as_deref"
        );
    }

    // -- field_label with multiple underscores --

    #[test]
    fn field_label_multiple_underscores() {
        assert_eq!(field_label("api_key_file_path"), "Api Key File Path");
    }

    // -- field_label single word --

    #[test]
    fn field_label_single_word() {
        assert_eq!(field_label("name"), "Name");
    }

    // -- is_display_field for Named type --

    #[test]
    fn is_display_field_named() {
        assert!(is_display_field(&make_field(
            "status",
            RustType::Named("Status".into()),
            true
        )));
    }

    // -- is_display_field for F64 and U64 --

    #[test]
    fn is_display_field_numeric_types() {
        assert!(is_display_field(&make_field("price", RustType::F64, true)));
        assert!(is_display_field(&make_field("count", RustType::U64, true)));
    }

    // -- Compact format limits to 4 fields --

    #[test]
    fn compact_format_limits_to_4_key_fields() {
        let inner = make_struct(
            "BigItem",
            vec![
                make_field("a", RustType::String, true),
                make_field("b", RustType::String, true),
                make_field("c", RustType::String, true),
                make_field("d", RustType::String, true),
                make_field("e", RustType::String, true),
                make_field("f", RustType::String, true),
            ],
        );
        let list_resp = make_struct(
            "BigList",
            vec![make_field(
                "items",
                RustType::Vec(Box::new(RustType::Named("BigItem".into()))),
                true,
            )],
        );
        let op = make_get_op_with_response("list_big", RustType::Named("BigList".into()));
        let spec = make_spec(vec![inner, list_resp], vec![op]);
        let code = generate(&spec);
        let pipe_count = code.matches(" | ").count();
        assert!(
            pipe_count <= 3,
            "compact format should show at most 4 fields (3 pipes), got {pipe_count}"
        );
    }

    // -- cursor field detection with "next" in name --

    #[test]
    fn cursor_field_detected_by_next_in_name() {
        let item = make_struct("Item", vec![make_field("id", RustType::I64, true)]);
        let list_resp = make_struct(
            "ItemList",
            vec![
                make_field(
                    "items",
                    RustType::Vec(Box::new(RustType::Named("Item".into()))),
                    true,
                ),
                make_field(
                    "next_page",
                    RustType::Option(Box::new(RustType::String)),
                    false,
                ),
            ],
        );
        let op = make_get_op_with_response("list_items", RustType::Named("ItemList".into()));
        let spec = make_spec(vec![item, list_resp], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("next_page"), "should detect 'next' field");
    }

    // -- Vec<Named> with unknown inner type uses Debug --

    #[test]
    fn list_format_unknown_inner_type_uses_debug() {
        let list_resp = make_struct(
            "MysteryList",
            vec![make_field(
                "items",
                RustType::Vec(Box::new(RustType::Named("Mystery".into()))),
                true,
            )],
        );
        let op =
            make_get_op_with_response("list_mystery", RustType::Named("MysteryList".into()));
        let spec = make_spec(vec![list_resp], vec![op]);
        let code = generate(&spec);
        assert!(code.contains("{:?}"), "unknown inner type should use Debug");
    }

    // -- Non-list cursor field is ignored (cursor requires Option) --

    #[test]
    fn non_option_cursor_field_not_printed() {
        let item = make_struct("Item", vec![make_field("id", RustType::I64, true)]);
        let list_resp = make_struct(
            "ItemList",
            vec![
                make_field(
                    "items",
                    RustType::Vec(Box::new(RustType::Named("Item".into()))),
                    true,
                ),
                make_field("cursor", RustType::String, true),
            ],
        );
        let op = make_get_op_with_response("list_items", RustType::Named("ItemList".into()));
        let spec = make_spec(vec![item, list_resp], vec![op]);
        let code = generate(&spec);
        assert!(
            !code.contains("next page"),
            "non-Option cursor field should not generate pagination text"
        );
    }
}
