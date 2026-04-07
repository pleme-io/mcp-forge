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
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ApiSpec, AuthMethod, FieldDef, OpRequestBody};

    fn make_spec(operations: Vec<Operation>) -> ApiSpec {
        ApiSpec {
            name: "TestApi".into(),
            description: Some("Test API for unit tests.".into()),
            version: "1.0.0".into(),
            base_url: Some("https://api.example.com".into()),
            auth: AuthMethod::Bearer,
            operations,
            types: vec![],
        }
    }

    fn make_get_op(id: &str, path: &str) -> Operation {
        Operation {
            id: id.into(),
            method: HttpMethod::Get,
            path: path.into(),
            summary: Some(format!("Get {id}")),
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        }
    }

    // -- Top-level structure --

    #[test]
    fn generates_rmcp_imports() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("use rmcp::"));
        assert!(code.contains("ServerHandler"));
        assert!(code.contains("ToolRouter"));
    }

    #[test]
    fn generates_mcp_struct() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("struct TestApiMcp {"));
        assert!(code.contains("client: TestApiClient,"));
        assert!(code.contains("tool_router: ToolRouter<Self>,"));
    }

    #[test]
    fn generates_tool_router_annotation() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[tool_router]"));
    }

    #[test]
    fn generates_server_handler_impl() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[tool_handler]"));
        assert!(code.contains("impl ServerHandler for TestApiMcp {"));
        assert!(code.contains("fn get_info(&self) -> ServerInfo {"));
    }

    #[test]
    fn generates_server_instructions() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("Test API for unit tests."));
    }

    #[test]
    fn generates_entry_point() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn run()"));
        assert!(code.contains("TestApiMcp::new()?"));
        assert!(code.contains("serve(stdio())"));
    }

    #[test]
    fn generates_constructor_with_config() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("fn new() -> Result<Self, String>"));
        assert!(code.contains("TestApiConfig::load()"));
        assert!(code.contains("auth::resolve_api_key"));
        assert!(code.contains("TestApiClient::new"));
    }

    // -- Input structs --

    #[test]
    fn generates_input_struct_for_parameterized_op() {
        let op = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{id}".into(),
            summary: Some("Get an item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: Some("The item ID".into()),
            }],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("struct GetItemInput {"));
        assert!(code.contains("id: String,"));
        assert!(code.contains("#[schemars(description = \"The item ID\")]"));
    }

    #[test]
    fn skips_empty_input_struct() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        // No ListItemsInput struct should be generated
        assert!(!code.contains("ListItemsInput {"));
        // But the tool method should still exist with serde_json::Value
        assert!(code.contains("Parameters<serde_json::Value>"));
    }

    #[test]
    fn generates_input_struct_with_body_fields() {
        let op = Operation {
            id: "create_item".into(),
            method: HttpMethod::Post,
            path: "/items".into(),
            summary: Some("Create an item".into()),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "name".into(),
                    rust_name: "name".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: Some("Item name".into()),
                    default_value: None,
                }],
                type_name: Some("CreateItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("struct CreateItemInput {"));
        assert!(code.contains("name: String,"));
    }

    // -- Tool methods --

    #[test]
    fn generates_tool_annotation_with_description() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("#[tool(description = \"Get list_items\")]"));
    }

    #[test]
    fn generates_tool_method_for_get() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("async fn list_items("));
        assert!(code.contains("match self.client.list_items("));
        assert!(code.contains("format::format_list_items(&result)"));
    }

    #[test]
    fn delete_operations_use_simple_success_message() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: Some("Delete an item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("Ok(_) => format!(\"Success: DELETE /items/{id}\")"));
    }

    #[test]
    fn generates_error_handling() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("Err(e) => format!(\"Error: {e}\")"));
    }

    #[test]
    fn generates_request_body_construction() {
        let op = Operation {
            id: "create_item".into(),
            method: HttpMethod::Post,
            path: "/items".into(),
            summary: Some("Create an item".into()),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![
                    FieldDef {
                        name: "name".into(),
                        rust_name: "name".into(),
                        rust_type: RustType::String,
                        required: true,
                        description: None,
                        default_value: None,
                    },
                    FieldDef {
                        name: "count".into(),
                        rust_name: "count".into(),
                        rust_type: RustType::I64,
                        required: true,
                        description: None,
                        default_value: None,
                    },
                ],
                type_name: Some("CreateItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("let req = crate::api::types::CreateItemRequest {"));
        assert!(code.contains("name: input.name.clone(),"));
        assert!(code.contains("count: input.count.clone(),"));
        assert!(code.contains("&req"));
    }

    // -- Helper function tests --

    #[test]
    fn escape_string_handles_quotes() {
        assert_eq!(escape_string("say \"hello\""), "say \\\"hello\\\"");
    }

    #[test]
    fn escape_string_handles_backslashes() {
        assert_eq!(escape_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_string_handles_newlines() {
        assert_eq!(escape_string("line1\nline2"), "line1 line2");
    }

    #[test]
    fn escape_string_no_change_for_simple() {
        assert_eq!(escape_string("simple text"), "simple text");
    }

    #[test]
    fn input_field_type_required_string() {
        assert_eq!(input_field_type(&RustType::String, true), "String");
    }

    #[test]
    fn input_field_type_optional_wraps() {
        assert_eq!(
            input_field_type(&RustType::String, false),
            "Option<String>"
        );
    }

    #[test]
    fn input_field_type_already_option_not_double_wrapped() {
        assert_eq!(
            input_field_type(&RustType::Option(Box::new(RustType::String)), false),
            "Option<String>"
        );
    }

    #[test]
    fn rust_type_string_all_variants() {
        assert_eq!(rust_type_string(&RustType::String), "String");
        assert_eq!(rust_type_string(&RustType::I64), "i64");
        assert_eq!(rust_type_string(&RustType::U64), "u64");
        assert_eq!(rust_type_string(&RustType::F64), "f64");
        assert_eq!(rust_type_string(&RustType::Bool), "bool");
        assert_eq!(rust_type_string(&RustType::Value), "serde_json::Value");
        assert_eq!(
            rust_type_string(&RustType::Vec(Box::new(RustType::I64))),
            "Vec<i64>"
        );
        assert_eq!(
            rust_type_string(&RustType::Named("Foo".into())),
            "Foo"
        );
    }

    #[test]
    fn is_option_type_true() {
        assert!(is_option_type(&RustType::Option(Box::new(RustType::String))));
    }

    #[test]
    fn is_option_type_false() {
        assert!(!is_option_type(&RustType::String));
        assert!(!is_option_type(&RustType::Vec(Box::new(RustType::String))));
    }

    #[test]
    fn default_description_fallback() {
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
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        // Fallback description: "do_thing operation"
        assert!(code.contains("do_thing operation"));
    }

    #[test]
    fn default_instructions_fallback() {
        let mut spec = make_spec(vec![]);
        spec.description = None;
        let code = generate(&spec);
        assert!(code.contains("TestApi MCP server"));
    }

    // -- Description fallback to operation description when no summary --

    #[test]
    fn tool_description_from_description_field() {
        let op = Operation {
            id: "do_thing".into(),
            method: HttpMethod::Post,
            path: "/thing".into(),
            summary: None,
            description: Some("A long description for this operation".into()),
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("A long description for this operation"));
    }

    // -- Mixed path + query + body params --

    #[test]
    fn tool_with_mixed_params() {
        let op = Operation {
            id: "update_user_setting".into(),
            method: HttpMethod::Put,
            path: "/users/{userId}/settings".into(),
            summary: Some("Update a user setting".into()),
            description: None,
            parameters: vec![
                OpParameter {
                    name: "userId".into(),
                    rust_name: "user_id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: Some("User identifier".into()),
                },
                OpParameter {
                    name: "force".into(),
                    rust_name: "force".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::Bool)),
                    description: None,
                },
            ],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "value".into(),
                    rust_name: "value".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: Some("Setting value".into()),
                    default_value: None,
                }],
                type_name: Some("UpdateSettingRequest".into()),
            }),
            response_type: Some(RustType::Named("Setting".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("struct UpdateUserSettingInput {"));
        assert!(code.contains("user_id: String,"));
        assert!(code.contains("force: Option<bool>,"));
        assert!(code.contains("value: String,"));
        assert!(code.contains("&input.user_id"));
    }

    // -- Query option in tool method uses as_deref --

    #[test]
    fn query_option_uses_as_deref_in_tool() {
        let op = Operation {
            id: "search".into(),
            method: HttpMethod::Get,
            path: "/search".into(),
            summary: Some("Search".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "q".into(),
                rust_name: "q".into(),
                location: ParamLocation::Query,
                required: false,
                rust_type: RustType::Option(Box::new(RustType::String)),
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("input.q.as_deref()"),
            "Option query param should use as_deref in tool method"
        );
    }

    // -- Non-option query param passed directly --

    #[test]
    fn query_non_option_passed_directly_in_tool() {
        let op = Operation {
            id: "list_items".into(),
            method: HttpMethod::Get,
            path: "/items".into(),
            summary: Some("List items".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "limit".into(),
                rust_name: "limit".into(),
                location: ParamLocation::Query,
                required: true,
                rust_type: RustType::I64,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("input.limit"),
            "required query param should be passed directly"
        );
    }

    // -- Header params are NOT included in MCP input struct --

    #[test]
    fn header_params_excluded_from_input_struct() {
        let op = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{id}".into(),
            summary: Some("Get item".into()),
            description: None,
            parameters: vec![
                OpParameter {
                    name: "id".into(),
                    rust_name: "id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: None,
                },
                OpParameter {
                    name: "X-Request-Id".into(),
                    rust_name: "x_request_id".into(),
                    location: ParamLocation::Header,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::String)),
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("id: String,"));
        assert!(
            !code.contains("x_request_id"),
            "header params should not appear in MCP input struct"
        );
    }

    // -- Escape string handles combined special chars --

    #[test]
    fn escape_string_combined() {
        assert_eq!(
            escape_string("say \"hello\"\nand\\goodbye"),
            "say \\\"hello\\\" and\\\\goodbye"
        );
    }

    // -- input_field_type with Vec --

    #[test]
    fn input_field_type_vec() {
        assert_eq!(
            input_field_type(&RustType::Vec(Box::new(RustType::String)), true),
            "Vec<String>"
        );
        assert_eq!(
            input_field_type(&RustType::Vec(Box::new(RustType::String)), false),
            "Option<Vec<String>>"
        );
    }

    // -- MCP struct derives Debug and Clone --

    #[test]
    fn mcp_struct_has_derive() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[derive(Debug, Clone)]"));
    }

    // -- Multiple operations generate multiple tools --

    #[test]
    fn multiple_operations_generate_multiple_tools() {
        let op1 = make_get_op("list_items", "/items");
        let op2 = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{id}".into(),
            summary: Some("Get item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op1, op2]);
        let code = generate(&spec);
        assert!(code.contains("async fn list_items("));
        assert!(code.contains("async fn get_item("));
    }

    // -- stop_ prefix uses simple success message --

    #[test]
    fn stop_operations_use_simple_success_message() {
        let op = Operation {
            id: "stop_service".into(),
            method: HttpMethod::Post,
            path: "/services/{id}/stop".into(),
            summary: Some("Stop a service".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("Ok(_) => format!(\"Success:"),
            "stop_ prefixed operations should use simple success message"
        );
    }

    // -- Request body construction with no type_name uses fallback --

    #[test]
    fn request_body_construction_fallback_name() {
        let op = Operation {
            id: "send_data".into(),
            method: HttpMethod::Post,
            path: "/data".into(),
            summary: Some("Send data".into()),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "payload".into(),
                    rust_name: "payload".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: None,
                    default_value: None,
                }],
                type_name: None,
            }),
            response_type: Some(RustType::Named("Result".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("SendDataRequest"));
    }
}
