use crate::ir::{ApiSpec, AuthMethod, HttpMethod, OpParameter, Operation, ParamLocation, RustType};
use heck::{ToSnakeCase, ToUpperCamelCase};

/// Generate the `src/client.rs` file from the API spec.
///
/// Produces a typed HTTP client struct with:
/// - Auth based on `spec.auth` (Bearer, Basic, ApiKeyHeader, None)
/// - Typed async methods for each operation
/// - Path parameter interpolation and query parameter URL-encoding
pub fn generate(spec: &ApiSpec) -> String {
    let mut out = String::with_capacity(16384);

    let pascal = spec.name.to_upper_camel_case();
    let client_name = format!("{pascal}Client");
    let error_name = format!("{pascal}Error");

    // Imports
    out.push_str(&format!(
        "use crate::api::types::*;\n\
         use crate::error::{{{error_name}, Result}};\n\
         \n"
    ));

    // Doc comment
    out.push_str(&format!(
        "/// HTTP client for the {} API.\n",
        spec.name
    ));
    if let Some(ref desc) = spec.description {
        out.push_str(&format!("///\n/// {desc}\n"));
    }

    // Struct definition
    out.push_str("#[derive(Debug, Clone)]\n");
    out.push_str(&format!("pub struct {client_name} {{\n"));
    out.push_str("    inner: reqwest::Client,\n");
    out.push_str("    base_url: String,\n");
    out.push_str("    api_key: String,\n");
    out.push_str("}\n\n");

    // Impl block
    out.push_str(&format!("impl {client_name} {{\n"));

    // Constructor
    let user_agent = format!(
        "pleme-io/{} {}",
        spec.name.to_snake_case(),
        spec.version
    );
    out.push_str(&format!(
        "    /// Create a new client.\n\
         \x20   pub fn new(base_url: &str, api_key: &str) -> Result<Self> {{\n\
         \x20       let inner = reqwest::Client::builder()\n\
         \x20           .timeout(std::time::Duration::from_secs(60))\n\
         \x20           .user_agent(\"{user_agent}\")\n\
         \x20           .build()\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \n\
         \x20       Ok(Self {{\n\
         \x20           inner,\n\
         \x20           base_url: base_url.trim_end_matches('/').to_string(),\n\
         \x20           api_key: api_key.to_string(),\n\
         \x20       }})\n\
         \x20   }}\n\
         \n"
    ));

    // URL helper
    out.push_str(
        "    fn url(&self, path: &str) -> String {\n\
         \x20       format!(\"{}/{}\", self.base_url, path.trim_start_matches('/'))\n\
         \x20   }\n\
         \n",
    );

    // Private HTTP method helpers (get, post, put, patch, delete)
    generate_http_helpers(&mut out, &spec.auth, &error_name);

    // Response handler
    out.push_str(&format!(
        "    async fn handle_response<T: serde::de::DeserializeOwned>(\n\
         \x20       resp: reqwest::Response,\n\
         \x20   ) -> Result<T> {{\n\
         \x20       let status = resp.status().as_u16();\n\
         \x20       if !resp.status().is_success() {{\n\
         \x20           let body = resp.text().await.unwrap_or_default();\n\
         \x20           return Err({error_name}::Api {{ status, body }});\n\
         \x20       }}\n\
         \x20       let text = resp.text().await.map_err({error_name}::Request)?;\n\
         \x20       serde_json::from_str(&text).map_err({error_name}::Json)\n\
         \x20   }}\n\
         \n"
    ));

    // Separator
    out.push_str("    // -- Public API methods --\n\n");

    // Generate a method for each operation
    for op in &spec.operations {
        generate_operation_method(&mut out, op);
    }

    out.push_str("}\n");

    out
}

fn auth_call(auth: &AuthMethod) -> String {
    match auth {
        AuthMethod::Bearer => ".bearer_auth(&self.api_key)".into(),
        AuthMethod::Basic => ".basic_auth(&self.api_key, Option::<&str>::None)".into(),
        AuthMethod::ApiKeyHeader(header) => {
            format!(".header(\"{header}\", &self.api_key)")
        }
        AuthMethod::None => String::new(),
    }
}

fn generate_http_helpers(out: &mut String, auth: &AuthMethod, error_name: &str) {
    let auth_call = auth_call(auth);

    // GET
    out.push_str(&format!(
        "    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {{\n\
         \x20       let resp = self\n\
         \x20           .inner\n\
         \x20           .get(&self.url(path))\n\
         \x20           {auth_call}\n\
         \x20           .send()\n\
         \x20           .await\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \x20       Self::handle_response(resp).await\n\
         \x20   }}\n\
         \n"
    ));

    // POST with body
    out.push_str(&format!(
        "    async fn post<B: serde::Serialize, T: serde::de::DeserializeOwned>(\n\
         \x20       &self,\n\
         \x20       path: &str,\n\
         \x20       body: &B,\n\
         \x20   ) -> Result<T> {{\n\
         \x20       let resp = self\n\
         \x20           .inner\n\
         \x20           .post(&self.url(path))\n\
         \x20           {auth_call}\n\
         \x20           .json(body)\n\
         \x20           .send()\n\
         \x20           .await\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \x20       Self::handle_response(resp).await\n\
         \x20   }}\n\
         \n"
    ));

    // POST without body
    out.push_str(&format!(
        "    async fn post_empty<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {{\n\
         \x20       let resp = self\n\
         \x20           .inner\n\
         \x20           .post(&self.url(path))\n\
         \x20           {auth_call}\n\
         \x20           .send()\n\
         \x20           .await\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \x20       Self::handle_response(resp).await\n\
         \x20   }}\n\
         \n"
    ));

    // PUT with body
    out.push_str(&format!(
        "    async fn put<B: serde::Serialize, T: serde::de::DeserializeOwned>(\n\
         \x20       &self,\n\
         \x20       path: &str,\n\
         \x20       body: &B,\n\
         \x20   ) -> Result<T> {{\n\
         \x20       let resp = self\n\
         \x20           .inner\n\
         \x20           .put(&self.url(path))\n\
         \x20           {auth_call}\n\
         \x20           .json(body)\n\
         \x20           .send()\n\
         \x20           .await\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \x20       Self::handle_response(resp).await\n\
         \x20   }}\n\
         \n"
    ));

    // PATCH with body
    out.push_str(&format!(
        "    async fn patch<B: serde::Serialize, T: serde::de::DeserializeOwned>(\n\
         \x20       &self,\n\
         \x20       path: &str,\n\
         \x20       body: &B,\n\
         \x20   ) -> Result<T> {{\n\
         \x20       let resp = self\n\
         \x20           .inner\n\
         \x20           .patch(&self.url(path))\n\
         \x20           {auth_call}\n\
         \x20           .json(body)\n\
         \x20           .send()\n\
         \x20           .await\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \x20       Self::handle_response(resp).await\n\
         \x20   }}\n\
         \n"
    ));

    // DELETE
    out.push_str(&format!(
        "    async fn delete<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {{\n\
         \x20       let resp = self\n\
         \x20           .inner\n\
         \x20           .delete(&self.url(path))\n\
         \x20           {auth_call}\n\
         \x20           .send()\n\
         \x20           .await\n\
         \x20           .map_err({error_name}::Request)?;\n\
         \x20       Self::handle_response(resp).await\n\
         \x20   }}\n\
         \n"
    ));
}

fn generate_operation_method(out: &mut String, op: &Operation) {
    let method_name = op.id.to_snake_case();

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

    let has_body = op.request_body.is_some();

    // Determine response type string
    let response_type = op
        .response_type
        .as_ref()
        .map(rust_type_to_string)
        .unwrap_or_else(|| "serde_json::Value".into());

    // Doc comment
    out.push_str(&format!("    /// {} {}\n", op.method, op.path));
    if let Some(ref summary) = op.summary {
        out.push_str(&format!("    ///\n    /// {summary}\n"));
    }

    // Method signature
    out.push_str(&format!("    pub async fn {method_name}(\n"));
    out.push_str("        &self,\n");

    // Path parameters as &str
    for param in &path_params {
        out.push_str(&format!("        {}: &str,\n", param.rust_name));
    }

    // Query parameters (required ones as their type, optional as Option)
    for param in &query_params {
        let type_str = param_type_string(param);
        out.push_str(&format!("        {}: {type_str},\n", param.rust_name));
    }

    // Request body (if any)
    if has_body {
        let body_type = request_body_type_name(op);
        out.push_str(&format!("        req: &{body_type},\n"));
    }

    out.push_str(&format!("    ) -> Result<{response_type}> {{\n"));

    // Build path string with interpolation
    let has_queries = !query_params.is_empty();
    if path_params.is_empty() && !has_queries {
        // Simple static path
        if has_body {
            out.push_str(&format!(
                "        self.{}(\"{}\", req).await\n",
                http_method_fn(&op.method, has_body),
                op.path
            ));
        } else {
            out.push_str(&format!(
                "        self.{}(\"{}\").await\n",
                http_method_fn(&op.method, has_body),
                op.path
            ));
        }
    } else {
        // Build path with interpolation
        let mut path_template = op.path.clone();
        for param in &path_params {
            // Replace {param_name} with format interpolation
            path_template = path_template.replace(
                &format!("{{{}}}", param.name),
                &format!("{{{}}}", param.rust_name),
            );
        }

        if has_queries {
            // Build path with query parameters
            out.push_str(&format!(
                "        let mut path = format!(\"{path_template}\");\n"
            ));

            let mut first_query = true;
            for param in &query_params {
                let separator = if first_query { '?' } else { '&' };
                first_query = false;

                if is_option_type(&param.rust_type) {
                    out.push_str(&format!(
                        "        if let Some(ref v) = {} {{\n\
                         \x20           path.push_str(&format!(\"{}{}={{}}\", urlencoding::encode(&v.to_string())));\n\
                         \x20       }}\n",
                        param.rust_name, separator, param.name
                    ));
                } else {
                    out.push_str(&format!(
                        "        path.push_str(&format!(\"{}{}={{}}\", urlencoding::encode(&{}.to_string())));\n",
                        separator, param.name, param.rust_name
                    ));
                }
            }

            if has_body {
                out.push_str(&format!(
                    "        self.{}(&path, req).await\n",
                    http_method_fn(&op.method, has_body)
                ));
            } else {
                out.push_str(&format!(
                    "        self.{}(&path).await\n",
                    http_method_fn(&op.method, has_body)
                ));
            }
        } else {
            // Path params only, no query params
            if has_body {
                out.push_str(&format!(
                    "        self.{}(&format!(\"{path_template}\"), req).await\n",
                    http_method_fn(&op.method, has_body)
                ));
            } else {
                out.push_str(&format!(
                    "        self.{}(&format!(\"{path_template}\")).await\n",
                    http_method_fn(&op.method, has_body)
                ));
            }
        }
    }

    out.push_str("    }\n\n");
}

fn http_method_fn(method: &HttpMethod, has_body: bool) -> &'static str {
    match method {
        HttpMethod::Get => "get",
        HttpMethod::Post => {
            if has_body {
                "post"
            } else {
                "post_empty"
            }
        }
        HttpMethod::Put => "put",
        HttpMethod::Patch => "patch",
        HttpMethod::Delete => "delete",
    }
}

fn param_type_string(param: &OpParameter) -> String {
    if param.required {
        match &param.rust_type {
            RustType::String => "&str".into(),
            RustType::Option(inner) => rust_type_to_string(inner),
            other => rust_type_to_string(other),
        }
    } else {
        match &param.rust_type {
            RustType::Option(_) => rust_type_to_string(&param.rust_type),
            _ => format!("Option<{}>", rust_type_to_string(&param.rust_type)),
        }
    }
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

fn is_option_type(rt: &RustType) -> bool {
    matches!(rt, RustType::Option(_))
}

fn request_body_type_name(op: &Operation) -> String {
    // If the request body has a named type, use it
    if let Some(ref body) = op.request_body {
        if let Some(ref name) = body.type_name {
            return name.clone();
        }
    }
    // Fallback: operation id in PascalCase + "Request"
    use heck::ToUpperCamelCase;
    format!("{}Request", op.id.to_upper_camel_case())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ApiSpec, AuthMethod, FieldDef, OpRequestBody};

    fn make_spec(
        name: &str,
        auth: AuthMethod,
        operations: Vec<Operation>,
    ) -> ApiSpec {
        ApiSpec {
            name: name.into(),
            description: None,
            version: "1.0.0".into(),
            base_url: Some("https://api.example.com".into()),
            auth,
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

    fn make_post_op(id: &str, path: &str) -> Operation {
        Operation {
            id: id.into(),
            method: HttpMethod::Post,
            path: path.into(),
            summary: Some(format!("Create {id}")),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "name".into(),
                    rust_name: "name".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: None,
                    default_value: None,
                }],
                type_name: Some("CreateItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        }
    }

    // -- Struct and constructor --

    #[test]
    fn generates_client_struct() {
        let spec = make_spec("TestApi", AuthMethod::Bearer, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub struct TestApiClient {"));
        assert!(code.contains("inner: reqwest::Client,"));
        assert!(code.contains("base_url: String,"));
        assert!(code.contains("api_key: String,"));
    }

    #[test]
    fn generates_constructor() {
        let spec = make_spec("TestApi", AuthMethod::Bearer, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub fn new(base_url: &str, api_key: &str)"));
        assert!(code.contains("reqwest::Client::builder()"));
        assert!(code.contains("timeout(std::time::Duration::from_secs(60))"));
    }

    #[test]
    fn generates_url_helper() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("fn url(&self, path: &str) -> String"));
    }

    #[test]
    fn generates_handle_response() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("async fn handle_response"));
        assert!(code.contains("is_success()"));
        assert!(code.contains("TestApiError::Api"));
    }

    // -- Auth methods --

    #[test]
    fn bearer_auth_call() {
        assert_eq!(
            auth_call(&AuthMethod::Bearer),
            ".bearer_auth(&self.api_key)"
        );
    }

    #[test]
    fn basic_auth_call() {
        assert_eq!(
            auth_call(&AuthMethod::Basic),
            ".basic_auth(&self.api_key, Option::<&str>::None)"
        );
    }

    #[test]
    fn api_key_header_auth_call() {
        assert_eq!(
            auth_call(&AuthMethod::ApiKeyHeader("X-Key".into())),
            ".header(\"X-Key\", &self.api_key)"
        );
    }

    #[test]
    fn no_auth_call() {
        assert_eq!(auth_call(&AuthMethod::None), "");
    }

    #[test]
    fn http_helpers_include_bearer_auth() {
        let spec = make_spec("TestApi", AuthMethod::Bearer, vec![]);
        let code = generate(&spec);
        assert!(code.contains(".bearer_auth(&self.api_key)"));
    }

    #[test]
    fn http_helpers_include_api_key_header() {
        let spec = make_spec("MyApi", AuthMethod::ApiKeyHeader("X-Api-Key".into()), vec![]);
        let code = generate(&spec);
        assert!(code.contains(".header(\"X-Api-Key\", &self.api_key)"));
    }

    // -- Operation methods --

    #[test]
    fn generates_get_method() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn list_items("));
        assert!(code.contains("-> Result<Item>"));
        assert!(code.contains("self.get(\"/items\").await"));
    }

    #[test]
    fn generates_post_method_with_body() {
        let op = make_post_op("create_item", "/items");
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn create_item("));
        assert!(code.contains("req: &CreateItemRequest,"));
        assert!(code.contains("self.post(\"/items\", req).await"));
    }

    #[test]
    fn generates_delete_method() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: None,
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
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn delete_item("));
        assert!(code.contains("id: &str,"));
        assert!(code.contains("self.delete("));
    }

    #[test]
    fn generates_path_parameter_interpolation() {
        let op = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{itemId}".into(),
            summary: None,
            description: None,
            parameters: vec![OpParameter {
                name: "itemId".into(),
                rust_name: "item_id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("item_id: &str,"));
        assert!(code.contains("format!(\"/items/{item_id}\")"));
    }

    #[test]
    fn generates_query_parameters() {
        let op = Operation {
            id: "list_items".into(),
            method: HttpMethod::Get,
            path: "/items".into(),
            summary: None,
            description: None,
            parameters: vec![
                OpParameter {
                    name: "limit".into(),
                    rust_name: "limit".into(),
                    location: ParamLocation::Query,
                    required: true,
                    rust_type: RustType::I64,
                    description: None,
                },
                OpParameter {
                    name: "cursor".into(),
                    rust_name: "cursor".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::String)),
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("limit: i64,"));
        assert!(code.contains("cursor: Option<String>,"));
        assert!(code.contains("urlencoding::encode"));
    }

    #[test]
    fn generates_doc_comment_for_operation() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("/// GET /items"));
        assert!(code.contains("/// Get list_items"));
    }

    #[test]
    fn response_type_fallback_to_value() {
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
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("-> Result<serde_json::Value>"));
    }

    // -- Helper function tests --

    #[test]
    fn http_method_fn_get() {
        assert_eq!(http_method_fn(&HttpMethod::Get, false), "get");
    }

    #[test]
    fn http_method_fn_post_with_body() {
        assert_eq!(http_method_fn(&HttpMethod::Post, true), "post");
    }

    #[test]
    fn http_method_fn_post_without_body() {
        assert_eq!(http_method_fn(&HttpMethod::Post, false), "post_empty");
    }

    #[test]
    fn http_method_fn_put() {
        assert_eq!(http_method_fn(&HttpMethod::Put, true), "put");
    }

    #[test]
    fn http_method_fn_patch() {
        assert_eq!(http_method_fn(&HttpMethod::Patch, true), "patch");
    }

    #[test]
    fn http_method_fn_delete() {
        assert_eq!(http_method_fn(&HttpMethod::Delete, false), "delete");
    }

    #[test]
    fn param_type_string_required_string() {
        let param = OpParameter {
            name: "name".into(),
            rust_name: "name".into(),
            location: ParamLocation::Query,
            required: true,
            rust_type: RustType::String,
            description: None,
        };
        assert_eq!(param_type_string(&param), "&str");
    }

    #[test]
    fn param_type_string_required_i64() {
        let param = OpParameter {
            name: "limit".into(),
            rust_name: "limit".into(),
            location: ParamLocation::Query,
            required: true,
            rust_type: RustType::I64,
            description: None,
        };
        assert_eq!(param_type_string(&param), "i64");
    }

    #[test]
    fn param_type_string_optional() {
        let param = OpParameter {
            name: "cursor".into(),
            rust_name: "cursor".into(),
            location: ParamLocation::Query,
            required: false,
            rust_type: RustType::Option(Box::new(RustType::String)),
            description: None,
        };
        assert_eq!(param_type_string(&param), "Option<String>");
    }

    #[test]
    fn request_body_type_name_from_type_name() {
        let op = make_post_op("create_item", "/items");
        assert_eq!(request_body_type_name(&op), "CreateItemRequest");
    }

    #[test]
    fn request_body_type_name_fallback() {
        let op = Operation {
            id: "update_item".into(),
            method: HttpMethod::Put,
            path: "/items".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![],
                type_name: None,
            }),
            response_type: None,
            errors: vec![],
        };
        assert_eq!(request_body_type_name(&op), "UpdateItemRequest");
    }

    #[test]
    fn client_description_included() {
        let mut spec = make_spec("TestApi", AuthMethod::None, vec![]);
        spec.description = Some("My test API description.".into());
        let code = generate(&spec);
        assert!(code.contains("My test API description."));
    }

    #[test]
    fn client_name_pascal_cased() {
        let spec = make_spec("my_api", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub struct MyApiClient"));
    }
}
