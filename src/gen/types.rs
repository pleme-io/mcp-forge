use crate::ir::{ApiSpec, FieldDef, RustType, TypeDef};

/// Generate the `src/api/types.rs` file from the API spec.
///
/// Produces serde-compatible structs and enums with proper derive macros,
/// serde annotations, and `#[serde(flatten)] pub extra: serde_json::Value`
/// on response types.
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

    // Serde annotations for optional fields
    if !field.required {
        match &field.rust_type {
            RustType::Option(_) => {
                out.push_str(
                    "    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n",
                );
            }
            RustType::Vec(_) => {
                out.push_str("    #[serde(default)]\n");
            }
            _ => {
                out.push_str("    #[serde(default)]\n");
            }
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

/// Check if name->rust_name is a standard camelCase to snake_case conversion.
fn is_camel_to_snake(name: &str, rust_name: &str) -> bool {
    use heck::ToSnakeCase;
    name.to_snake_case() == *rust_name
}

/// Determine if any fields suggest camelCase API naming.
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
            .is_some_and(|rt| rust_type_contains_named(rt, rust_name))
    })
}

/// Check if a RustType references a specific named type.
fn rust_type_contains_named(rt: &RustType, name: &str) -> bool {
    match rt {
        RustType::Named(n) => n == name,
        RustType::Vec(inner) | RustType::Option(inner) => rust_type_contains_named(inner, name),
        _ => false,
    }
}

/// Infer the serde rename_all strategy for enum variants.
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
