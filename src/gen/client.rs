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
