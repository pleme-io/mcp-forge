use crate::ir::{
    ApiSpec, HttpMethod, OpParameter, Operation, ParamLocation, RustType,
};
use heck::{ToSnakeCase, ToUpperCamelCase};

/// Generate the `src/mcp.rs` file from the API spec.
///
/// Produces:
/// - MCP input structs with `schemars::JsonSchema` for each operation
/// - An MCP server struct with `#[tool_router]` / `#[tool_handler]`
/// - Tool methods that delegate to the client and format results
pub fn generate(spec: &ApiSpec) -> String {
    let mut out = String::with_capacity(16384);

    let pascal = spec.name.to_upper_camel_case();
    let mcp_struct_name = format!("{pascal}Mcp");
    let client_type = format!("{pascal}Client");
    let config_type = format!("{pascal}Config");

    // Imports
    out.push_str(
        "use rmcp::{\n\
         \x20   ServerHandler, ServiceExt,\n\
         \x20   handler::server::{router::tool::ToolRouter, wrapper::Parameters},\n\
         \x20   model::{ServerCapabilities, ServerInfo},\n\
         \x20   schemars, tool, tool_handler, tool_router,\n\
         \x20   transport::stdio,\n\
         };\n\
         use serde::Deserialize;\n\
         \n\
         use crate::auth;\n",
    );
    out.push_str(&format!("use crate::client::{client_type};\n"));
    out.push_str(&format!("use crate::config::{config_type};\n"));
    out.push_str("use crate::format;\n\n");

    // Generate input structs for each operation
    out.push_str(
        "// -- MCP tool input types --\n\
         //\n\
         // Each struct maps to an operation from the API spec. Field descriptions\n\
         // are preserved for schemars -> MCP tool schema generation.\n\n",
    );

    for op in &spec.operations {
        generate_input_struct(&mut out, op);
    }

    // MCP Server struct
    out.push_str("// -- MCP Server --\n\n");

    out.push_str("#[derive(Debug, Clone)]\n");
    out.push_str(&format!("struct {mcp_struct_name} {{\n"));
    out.push_str(&format!("    client: {client_type},\n"));
    out.push_str("    tool_router: ToolRouter<Self>,\n");
    out.push_str("}\n\n");

    // tool_router impl
    out.push_str("#[tool_router]\n");
    out.push_str(&format!("impl {mcp_struct_name} {{\n"));

    // Constructor
    out.push_str(
        "    fn new() -> Result<Self, String> {\n",
    );
    out.push_str(&format!(
        "        let config = {config_type}::load();\n"
    ));
    out.push_str(
        "        let api_key = auth::resolve_api_key(None, &config).map_err(|e| e.to_string())?;\n",
    );
    out.push_str(&format!(
        "        let client =\n\
         \x20           {client_type}::new(&config.api_url, &api_key).map_err(|e| e.to_string())?;\n\
         \n"
    ));
    out.push_str(
        "        Ok(Self {\n\
         \x20           client,\n\
         \x20           tool_router: Self::tool_router(),\n\
         \x20       })\n\
         \x20   }\n\n",
    );

    // Generate a tool method for each operation
    for op in &spec.operations {
        generate_tool_method(&mut out, op);
    }

    out.push_str("}\n\n");

    // ServerHandler impl
    out.push_str("#[tool_handler]\n");
    out.push_str(&format!(
        "impl ServerHandler for {mcp_struct_name} {{\n"
    ));
    out.push_str("    fn get_info(&self) -> ServerInfo {\n");
    out.push_str("        ServerInfo {\n");
    out.push_str("            instructions: Some(\n");

    let default_instructions = format!("{} MCP server", spec.name);
    let instructions = spec
        .description
        .as_deref()
        .unwrap_or(&default_instructions);
    out.push_str(&format!(
        "                \"{}\"\n\
         \x20                   .into(),\n",
        escape_string(instructions)
    ));
    out.push_str(
        "            ),\n\
         \x20           capabilities: ServerCapabilities::builder().enable_tools().build(),\n\
         \x20           ..Default::default()\n\
         \x20       }\n\
         \x20   }\n",
    );
    out.push_str("}\n\n");

    // Entry point
    out.push_str(
        "// -- Entry point --\n\
         \n\
         pub async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {\n",
    );
    out.push_str(&format!(
        "    let server = {mcp_struct_name}::new()?.serve(stdio()).await?;\n"
    ));
    out.push_str(
        "    server.waiting().await?;\n\
         \x20   Ok(())\n\
         }\n",
    );

    out
}

fn generate_input_struct(out: &mut String, op: &Operation) {
    let struct_name = format!("{}Input", op.id.to_upper_camel_case());

    // Collect all parameters that should be in the input struct
    let params: Vec<&OpParameter> = op
        .parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Path || p.location == ParamLocation::Query)
        .collect();

    let body_fields = op.request_body.as_ref().map(|b| &b.fields);

    // Skip generating empty input structs (will use serde_json::Value)
    let has_params = !params.is_empty();
    let has_body_fields = body_fields.is_some_and(|f| !f.is_empty());

    if !has_params && !has_body_fields {
        return;
    }

    out.push_str("#[derive(Debug, Deserialize, schemars::JsonSchema)]\n");
    out.push_str(&format!("struct {struct_name} {{\n"));

    // Path/query parameters
    for param in &params {
        if let Some(ref desc) = param.description {
            out.push_str(&format!(
                "    #[schemars(description = \"{}\")]\n",
                escape_string(desc)
            ));
        }
        let field_type = input_field_type(&param.rust_type, param.required);
        out.push_str(&format!("    {}: {field_type},\n", param.rust_name));
    }

    // Request body fields
    if let Some(fields) = body_fields {
        for field in fields {
            if let Some(ref desc) = field.description {
                out.push_str(&format!(
                    "    #[schemars(description = \"{}\")]\n",
                    escape_string(desc)
                ));
            }
            let field_type = input_field_type(&field.rust_type, field.required);
            out.push_str(&format!("    {}: {field_type},\n", field.rust_name));
        }
    }

    out.push_str("}\n\n");
}

fn generate_tool_method(out: &mut String, op: &Operation) {
    let method_name = op.id.to_snake_case();
    let input_struct = format!("{}Input", op.id.to_upper_camel_case());

    // Tool description from summary/description
    let default_description = format!("{} operation", op.id);
    let description = op
        .summary
        .as_deref()
        .or(op.description.as_deref())
        .unwrap_or(&default_description);

    out.push_str(&format!(
        "    #[tool(description = \"{}\")]\n",
        escape_string(description)
    ));

    // Determine if this operation has any input parameters
    let has_params = !op.parameters.is_empty()
        || op
            .request_body
            .as_ref()
            .is_some_and(|b| !b.fields.is_empty());

    if has_params {
        out.push_str(&format!(
            "    async fn {method_name}(&self, Parameters(input): Parameters<{input_struct}>) -> String {{\n"
        ));
    } else {
        out.push_str(&format!(
            "    async fn {method_name}(&self, Parameters(_): Parameters<serde_json::Value>) -> String {{\n"
        ));
    }

    // Build the client method call
    let client_method = method_name.clone();

    // Collect path params
    let path_params: Vec<&OpParameter> = op
        .parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Path)
        .collect();

    let query_params: Vec<&OpParameter> = op
        .parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Query)
        .collect();

    let has_body = op
        .request_body
        .as_ref()
        .is_some_and(|b| !b.fields.is_empty());

    // Build client call arguments
    let mut args = Vec::new();

    for param in &path_params {
        args.push(format!("&input.{}", param.rust_name));
    }

    for param in &query_params {
        if is_option_type(&param.rust_type) {
            args.push(format!("input.{}.as_deref()", param.rust_name));
        } else {
            args.push(format!("input.{}", param.rust_name));
        }
    }

    if has_body {
        // Build the request body from input fields
        generate_request_body_construction(out, op);
        args.push("&req".into());
    }

    let args_str = args.join(", ");

    out.push_str(&format!(
        "        match self.client.{client_method}({args_str}).await {{\n"
    ));

    // Determine if this is a simple action (stop/delete) or has a rich response
    let is_simple_action = matches!(op.method, HttpMethod::Delete)
        || op.id.to_snake_case().starts_with("stop")
        || op.id.to_snake_case().starts_with("delete");

    if is_simple_action {
        out.push_str(&format!(
            "            Ok(_) => format!(\"Success: {} {}\"),\n",
            op.method, op.path
        ));
    } else {
        let format_fn = format!("format_{method_name}");
        out.push_str(&format!(
            "            Ok(result) => format::{format_fn}(&result),\n"
        ));
    }
    out.push_str("            Err(e) => format!(\"Error: {e}\"),\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
}

fn generate_request_body_construction(out: &mut String, op: &Operation) {
    if let Some(ref body) = op.request_body {
        let request_type = body
            .type_name
            .clone()
            .unwrap_or_else(|| {
                format!("{}Request", op.id.to_upper_camel_case())
            });
        out.push_str(&format!(
            "        let req = crate::api::types::{request_type} {{\n"
        ));
        for field in &body.fields {
            out.push_str(&format!(
                "            {}: input.{}.clone(),\n",
                field.rust_name, field.rust_name
            ));
        }
        out.push_str("        };\n");
    }
}

fn input_field_type(rt: &RustType, required: bool) -> String {
    if required {
        rust_type_string(rt)
    } else {
        match rt {
            RustType::Option(_) => rust_type_string(rt),
            _ => format!("Option<{}>", rust_type_string(rt)),
        }
    }
}

fn rust_type_string(rt: &RustType) -> String {
    match rt {
        RustType::String => "String".into(),
        RustType::I64 => "i64".into(),
        RustType::U64 => "u64".into(),
        RustType::F64 => "f64".into(),
        RustType::Bool => "bool".into(),
        RustType::Vec(inner) => format!("Vec<{}>", rust_type_string(inner)),
        RustType::Option(inner) => format!("Option<{}>", rust_type_string(inner)),
        RustType::Named(name) => name.clone(),
        RustType::Value => "serde_json::Value".into(),
    }
}

fn is_option_type(rt: &RustType) -> bool {
    matches!(rt, RustType::Option(_))
}

fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}
