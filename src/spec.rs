// OpenAPI 3.0.3 serde types — only what mcp-forge needs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── Root ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiSpec {
    pub info: Info,
    #[serde(default)]
    pub paths: BTreeMap<String, PathItem>,
    #[serde(default)]
    pub components: Option<Components>,
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub security: Vec<BTreeMap<String, Vec<String>>>,
}

// ── Info ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub version: String,
}

// ── Paths & Operations ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathItem {
    #[serde(default)]
    pub get: Option<Operation>,
    #[serde(default)]
    pub post: Option<Operation>,
    #[serde(default)]
    pub put: Option<Operation>,
    #[serde(default)]
    pub delete: Option<Operation>,
    #[serde(default)]
    pub patch: Option<Operation>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub request_body: Option<RequestBody>,
    #[serde(default)]
    pub responses: BTreeMap<String, Response>,
    #[serde(default)]
    pub security: Vec<BTreeMap<String, Vec<String>>>,
    #[serde(default)]
    pub tags: Vec<String>,
}

// ── Parameters ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "in")]
    pub location: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub schema: Option<Schema>,
    /// $ref pointer, e.g. "#/components/parameters/Foo"
    #[serde(rename = "$ref", default)]
    pub ref_path: Option<String>,
}

// ── Request / Response Bodies ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBody {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub content: BTreeMap<String, MediaType>,
    #[serde(default)]
    pub description: Option<String>,
    /// $ref pointer
    #[serde(rename = "$ref", default)]
    pub ref_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaType {
    #[serde(default)]
    pub schema: Option<Schema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub content: Option<BTreeMap<String, MediaType>>,
    /// $ref pointer
    #[serde(rename = "$ref", default)]
    pub ref_path: Option<String>,
}

// ── Schema ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Schema {
    #[serde(rename = "type", default)]
    pub schema_type: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, Schema>,
    #[serde(default)]
    pub items: Option<Box<Schema>>,
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(rename = "enum", default)]
    pub enum_values: Option<Vec<serde_json::Value>>,
    /// $ref pointer, e.g. "#/components/schemas/Foo"
    #[serde(rename = "$ref", default)]
    pub ref_path: Option<String>,
    #[serde(rename = "allOf", default)]
    pub all_of: Vec<Schema>,
    #[serde(rename = "oneOf", default)]
    pub one_of: Vec<Schema>,
    #[serde(rename = "anyOf", default)]
    pub any_of: Vec<Schema>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub minimum: Option<f64>,
    #[serde(default)]
    pub maximum: Option<f64>,
    #[serde(rename = "minLength", default)]
    pub min_length: Option<u64>,
    #[serde(rename = "maxLength", default)]
    pub max_length: Option<u64>,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub additional_properties: Option<Box<Schema>>,
    #[serde(default)]
    pub title: Option<String>,
}

// ── Components ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    #[serde(default)]
    pub schemas: BTreeMap<String, Schema>,
    #[serde(default)]
    pub security_schemes: BTreeMap<String, SecurityScheme>,
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(default)]
    pub request_bodies: BTreeMap<String, RequestBody>,
    #[serde(default)]
    pub responses: BTreeMap<String, Response>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScheme {
    #[serde(rename = "type")]
    pub scheme_type: String,
    #[serde(default)]
    pub scheme: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// For apiKey type: "header", "query", or "cookie"
    #[serde(rename = "in", default)]
    pub location: Option<String>,
    /// For apiKey type: the header/query parameter name
    #[serde(default)]
    pub name: Option<String>,
}

// ── Server ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
}

// ── $ref resolution helpers ────────────────────────────────────────────────

/// Extract the final component name from a JSON pointer.
///
/// ```text
/// "#/components/schemas/Pet" → "Pet"
/// "#/components/parameters/LimitParam" → "LimitParam"
/// ```
pub fn ref_name(ref_path: &str) -> &str {
    ref_path.rsplit('/').next().unwrap_or(ref_path)
}

impl OpenApiSpec {
    /// Look up a schema by `$ref` pointer like `#/components/schemas/Foo`.
    pub fn resolve_schema_ref(&self, ref_path: &str) -> Option<&Schema> {
        let name = ref_name(ref_path);
        self.components.as_ref()?.schemas.get(name)
    }

    /// Look up a parameter by `$ref` pointer like `#/components/parameters/Foo`.
    pub fn resolve_parameter_ref(&self, ref_path: &str) -> Option<&Parameter> {
        let name = ref_name(ref_path);
        self.components.as_ref()?.parameters.get(name)
    }

    /// Look up a request body by `$ref` pointer.
    pub fn resolve_request_body_ref(&self, ref_path: &str) -> Option<&RequestBody> {
        let name = ref_name(ref_path);
        self.components.as_ref()?.request_bodies.get(name)
    }

    /// Look up a response by `$ref` pointer.
    pub fn resolve_response_ref(&self, ref_path: &str) -> Option<&Response> {
        let name = ref_name(ref_path);
        self.components.as_ref()?.responses.get(name)
    }
}
