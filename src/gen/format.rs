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
