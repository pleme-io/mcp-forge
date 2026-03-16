// Intermediate representation derived from parsed OpenAPI specs.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use heck::{ToSnakeCase, ToUpperCamelCase};

use crate::spec::{self, OpenApiSpec, Schema};

// ── Top-level IR ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ApiSpec {
    pub name: String,
    pub description: Option<String>,
    pub version: String,
    pub base_url: Option<String>,
    pub auth: AuthMethod,
    pub operations: Vec<Operation>,
    pub types: Vec<TypeDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    Bearer,
    Basic,
    ApiKeyHeader(String),
    None,
}

// ── Operations ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Operation {
    pub id: String,
    pub method: HttpMethod,
    pub path: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub parameters: Vec<OpParameter>,
    pub request_body: Option<OpRequestBody>,
    pub response_type: Option<RustType>,
    pub errors: Vec<ErrorResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Get => write!(f, "GET"),
            Self::Post => write!(f, "POST"),
            Self::Put => write!(f, "PUT"),
            Self::Delete => write!(f, "DELETE"),
            Self::Patch => write!(f, "PATCH"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ErrorResponse {
    pub status_code: String,
    pub description: Option<String>,
}

// ── Parameters ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpParameter {
    pub name: String,
    pub rust_name: String,
    pub location: ParamLocation,
    pub required: bool,
    pub rust_type: RustType,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamLocation {
    Path,
    Query,
    Header,
}

// ── Request Body ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OpRequestBody {
    pub required: bool,
    pub fields: Vec<FieldDef>,
    /// If the body maps to a single named type, store that name.
    pub type_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub rust_name: String,
    pub rust_type: RustType,
    pub required: bool,
    pub description: Option<String>,
    pub default_value: Option<serde_json::Value>,
}

// ── Type Definitions ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub rust_name: String,
    pub fields: Vec<FieldDef>,
    pub is_enum: bool,
    pub enum_variants: Vec<EnumVariant>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub rust_name: String,
}

// ── Rust Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustType {
    String,
    I64,
    U64,
    F64,
    Bool,
    Vec(Box<RustType>),
    Option(Box<RustType>),
    Named(std::string::String),
    /// Fallback to `serde_json::Value` for untyped / mixed schemas.
    Value,
}

impl std::fmt::Display for RustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String => write!(f, "String"),
            Self::I64 => write!(f, "i64"),
            Self::U64 => write!(f, "u64"),
            Self::F64 => write!(f, "f64"),
            Self::Bool => write!(f, "bool"),
            Self::Vec(inner) => write!(f, "Vec<{inner}>"),
            Self::Option(inner) => write!(f, "Option<{inner}>"),
            Self::Named(n) => write!(f, "{n}"),
            Self::Value => write!(f, "serde_json::Value"),
        }
    }
}

// ── Conversion: OpenApiSpec → ApiSpec ──────────────────────────────────────

impl ApiSpec {
    pub fn from_openapi(spec: &OpenApiSpec) -> Result<Self> {
        let mut converter = Converter::new(spec);
        converter.run()
    }
}

/// Internal conversion state.
struct Converter<'a> {
    spec: &'a OpenApiSpec,
    types: Vec<TypeDef>,
    /// Track which component schemas we've already emitted to avoid duplicates.
    emitted: std::collections::HashSet<std::string::String>,
}

impl<'a> Converter<'a> {
    fn new(spec: &'a OpenApiSpec) -> Self {
        Self {
            spec,
            types: Vec::new(),
            emitted: std::collections::HashSet::new(),
        }
    }

    fn run(&mut self) -> Result<ApiSpec> {
        // 1. Emit all component schemas as TypeDefs first.
        self.convert_component_schemas();

        // 2. Convert operations.
        let operations = self.convert_operations()?;

        // 3. Detect auth.
        let auth = self.detect_auth();

        // 4. Base URL from first server.
        let base_url = self.spec.servers.first().map(|s| s.url.clone());

        Ok(ApiSpec {
            name: self.spec.info.title.clone(),
            description: self.spec.info.description.clone(),
            version: self.spec.info.version.clone(),
            base_url,
            auth,
            operations,
            types: self.types.clone(),
        })
    }

    // ── Auth detection ─────────────────────────────────────────────────

    fn detect_auth(&self) -> AuthMethod {
        let components = match &self.spec.components {
            Some(c) => c,
            None => return AuthMethod::None,
        };

        for scheme in components.security_schemes.values() {
            match scheme.scheme_type.as_str() {
                "http" => {
                    if let Some(s) = &scheme.scheme {
                        match s.to_lowercase().as_str() {
                            "bearer" => return AuthMethod::Bearer,
                            "basic" => return AuthMethod::Basic,
                            _ => {}
                        }
                    }
                }
                "apiKey" => {
                    if scheme.location.as_deref() == Some("header") {
                        if let Some(name) = &scheme.name {
                            return AuthMethod::ApiKeyHeader(name.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        AuthMethod::None
    }

    // ── Component schemas ──────────────────────────────────────────────

    fn convert_component_schemas(&mut self) {
        let schemas = match &self.spec.components {
            Some(c) => c.schemas.clone(),
            None => return,
        };

        for (name, schema) in &schemas {
            self.ensure_type(name, schema);
        }
    }

    /// Ensure a named type exists in `self.types`. Returns the Rust type name.
    fn ensure_type(&mut self, name: &str, schema: &Schema) -> std::string::String {
        let rust_name = name.to_upper_camel_case();
        if self.emitted.contains(&rust_name) {
            return rust_name;
        }
        self.emitted.insert(rust_name.clone());

        // Check if it's an enum.
        if let Some(enum_vals) = &schema.enum_values {
            let variants = enum_vals
                .iter()
                .filter_map(|v| v.as_str().map(std::string::String::from))
                .map(|s| EnumVariant {
                    rust_name: s.to_upper_camel_case(),
                    name: s,
                })
                .collect();

            self.types.push(TypeDef {
                name: name.to_string(),
                rust_name: rust_name.clone(),
                fields: Vec::new(),
                is_enum: true,
                enum_variants: variants,
                description: schema.description.clone(),
            });
            return rust_name;
        }

        // Handle allOf by merging properties.
        let merged = self.merge_all_of(schema);
        let working = if merged.properties.is_empty() && !schema.properties.is_empty() {
            schema
        } else if !merged.properties.is_empty() {
            &merged
        } else {
            schema
        };

        let fields = self.schema_to_fields(working);

        self.types.push(TypeDef {
            name: name.to_string(),
            rust_name: rust_name.clone(),
            fields,
            is_enum: false,
            enum_variants: Vec::new(),
            description: schema.description.clone(),
        });

        rust_name
    }

    /// Merge all `allOf` entries into a single schema (shallow property merge).
    fn merge_all_of(&self, schema: &Schema) -> Schema {
        if schema.all_of.is_empty() {
            return schema.clone();
        }
        let mut merged = Schema::default();
        merged.schema_type = Some("object".into());

        for sub in &schema.all_of {
            let resolved = if let Some(ref_path) = &sub.ref_path {
                self.spec
                    .resolve_schema_ref(ref_path)
                    .cloned()
                    .unwrap_or_default()
            } else {
                sub.clone()
            };
            for (k, v) in &resolved.properties {
                merged.properties.insert(k.clone(), v.clone());
            }
            for r in &resolved.required {
                if !merged.required.contains(r) {
                    merged.required.push(r.clone());
                }
            }
        }

        // Also pull in direct properties from the parent.
        for (k, v) in &schema.properties {
            merged.properties.insert(k.clone(), v.clone());
        }
        for r in &schema.required {
            if !merged.required.contains(r) {
                merged.required.push(r.clone());
            }
        }

        merged
    }

    /// Convert a schema's properties into `Vec<FieldDef>`.
    fn schema_to_fields(&mut self, schema: &Schema) -> Vec<FieldDef> {
        let mut fields = Vec::new();
        for (name, prop) in &schema.properties {
            let rust_name = name.to_snake_case();
            let required = schema.required.contains(name);
            let mut rust_type = self.schema_to_rust_type(prop, Some(name));

            if !required && !matches!(rust_type, RustType::Option(_)) {
                rust_type = RustType::Option(Box::new(rust_type));
            }

            fields.push(FieldDef {
                name: name.clone(),
                rust_name,
                rust_type,
                required,
                description: prop.description.clone(),
                default_value: prop.default.clone(),
            });
        }
        fields
    }

    // ── Schema → RustType ──────────────────────────────────────────────

    /// Convert an OpenAPI Schema to a `RustType`, optionally creating named
    /// sub-types for inline objects.
    fn schema_to_rust_type(
        &mut self,
        schema: &Schema,
        context_name: Option<&str>,
    ) -> RustType {
        // Handle $ref first.
        if let Some(ref_path) = &schema.ref_path {
            let name = spec::ref_name(ref_path);
            // Make sure the referenced type is emitted.
            if let Some(resolved) = self.spec.resolve_schema_ref(ref_path) {
                let resolved = resolved.clone();
                self.ensure_type(name, &resolved);
            }
            return RustType::Named(name.to_upper_camel_case());
        }

        // Handle allOf / oneOf / anyOf.
        if !schema.all_of.is_empty() {
            let merged = self.merge_all_of(schema);
            if let Some(ctx) = context_name {
                let type_name = ctx.to_upper_camel_case();
                self.ensure_type(ctx, &merged);
                return RustType::Named(type_name);
            }
            return RustType::Value;
        }
        if !schema.one_of.is_empty() || !schema.any_of.is_empty() {
            return RustType::Value;
        }

        // Handle enum at the field level — promote to a named type.
        if let Some(enum_vals) = &schema.enum_values {
            if let Some(ctx) = context_name {
                let enum_name = ctx.to_upper_camel_case();
                if !self.emitted.contains(&enum_name) {
                    self.emitted.insert(enum_name.clone());
                    let variants = enum_vals
                        .iter()
                        .filter_map(|v| v.as_str().map(std::string::String::from))
                        .map(|s| EnumVariant {
                            rust_name: s.to_upper_camel_case(),
                            name: s,
                        })
                        .collect();
                    self.types.push(TypeDef {
                        name: ctx.to_string(),
                        rust_name: enum_name.clone(),
                        fields: Vec::new(),
                        is_enum: true,
                        enum_variants: variants,
                        description: schema.description.clone(),
                    });
                }
                return RustType::Named(enum_name);
            }
            return RustType::String;
        }

        match schema.schema_type.as_deref() {
            Some("string") => RustType::String,
            Some("integer") => match schema.format.as_deref() {
                Some("int64") => RustType::I64,
                Some("uint64") => RustType::U64,
                _ => RustType::I64,
            },
            Some("number") => RustType::F64,
            Some("boolean") => RustType::Bool,
            Some("array") => {
                let inner = schema
                    .items
                    .as_ref()
                    .map(|s| self.schema_to_rust_type(s, context_name))
                    .unwrap_or(RustType::Value);
                RustType::Vec(Box::new(inner))
            }
            Some("object") => {
                if schema.properties.is_empty() {
                    // Untyped object — use Value.
                    if schema.additional_properties.is_some() {
                        return RustType::Value;
                    }
                    return RustType::Value;
                }
                // Inline object with properties — create a named sub-type.
                if let Some(ctx) = context_name {
                    let type_name = ctx.to_upper_camel_case();
                    self.ensure_type(ctx, schema);
                    return RustType::Named(type_name);
                }
                RustType::Value
            }
            _ => {
                // No explicit type. Check for properties (implicit object).
                if !schema.properties.is_empty() {
                    if let Some(ctx) = context_name {
                        let type_name = ctx.to_upper_camel_case();
                        self.ensure_type(ctx, schema);
                        return RustType::Named(type_name);
                    }
                }
                RustType::Value
            }
        }
    }

    // ── Operations ─────────────────────────────────────────────────────

    fn convert_operations(&mut self) -> Result<Vec<Operation>> {
        let mut ops = Vec::new();

        for (path, item) in &self.spec.paths {
            let path_params = &item.parameters;

            let methods: [(HttpMethod, &Option<spec::Operation>); 5] = [
                (HttpMethod::Get, &item.get),
                (HttpMethod::Post, &item.post),
                (HttpMethod::Put, &item.put),
                (HttpMethod::Delete, &item.delete),
                (HttpMethod::Patch, &item.patch),
            ];

            for (method, maybe_op) in &methods {
                if let Some(op) = maybe_op {
                    let converted = self
                        .convert_operation(*method, path, op, path_params)
                        .with_context(|| {
                            format!(
                                "converting {method} {path} (operationId: {:?})",
                                op.operation_id
                            )
                        })?;
                    ops.push(converted);
                }
            }
        }

        Ok(ops)
    }

    fn convert_operation(
        &mut self,
        method: HttpMethod,
        path: &str,
        op: &spec::Operation,
        path_level_params: &[spec::Parameter],
    ) -> Result<Operation> {
        let id = op
            .operation_id
            .clone()
            .unwrap_or_else(|| {
                format!("{}_{}", format!("{method}").to_lowercase(), path.replace('/', "_"))
            })
            .to_snake_case();

        // Merge path-level and operation-level parameters, operation wins.
        // We clone parameters to avoid holding borrows on `self` through
        // `resolve_parameter` while we later call `convert_parameter`.
        let mut param_map: BTreeMap<std::string::String, spec::Parameter> = BTreeMap::new();
        for p in path_level_params {
            let resolved = self.resolve_parameter(p).cloned();
            let p = resolved.as_ref().unwrap_or(p);
            param_map.insert(format!("{}:{}", p.location, p.name), p.clone());
        }
        for p in &op.parameters {
            let resolved = self.resolve_parameter(p).cloned();
            let p = resolved.as_ref().unwrap_or(p);
            param_map.insert(format!("{}:{}", p.location, p.name), p.clone());
        }

        let collected: Vec<spec::Parameter> = param_map.into_values().collect();
        let parameters: Vec<OpParameter> = collected
            .iter()
            .map(|p| self.convert_parameter(p))
            .collect();

        // Request body.
        let request_body = self.convert_request_body(op.request_body.as_ref(), &id)?;

        // Response type — take the first 2xx response with a schema.
        let response_type = self.extract_response_type(&op.responses);

        // Error responses (non-2xx).
        let errors: Vec<ErrorResponse> = op
            .responses
            .iter()
            .filter(|(code, _)| !code.starts_with('2'))
            .map(|(code, resp)| ErrorResponse {
                status_code: code.clone(),
                description: resp.description.clone(),
            })
            .collect();

        Ok(Operation {
            id,
            method,
            path: path.to_string(),
            summary: op.summary.clone(),
            description: op.description.clone(),
            parameters,
            request_body,
            response_type,
            errors,
        })
    }

    fn resolve_parameter<'b>(&'b self, param: &'b spec::Parameter) -> Option<&'b spec::Parameter> {
        param
            .ref_path
            .as_deref()
            .and_then(|r| self.spec.resolve_parameter_ref(r))
    }

    fn convert_parameter(&mut self, param: &spec::Parameter) -> OpParameter {
        let location = match param.location.as_str() {
            "path" => ParamLocation::Path,
            "header" => ParamLocation::Header,
            _ => ParamLocation::Query,
        };

        let mut rust_type = param
            .schema
            .as_ref()
            .map(|s| self.schema_to_rust_type(s, Some(&param.name)))
            .unwrap_or(RustType::String);

        // Path params are always required.
        let required = param.required || location == ParamLocation::Path;

        if !required && !matches!(rust_type, RustType::Option(_)) {
            rust_type = RustType::Option(Box::new(rust_type));
        }

        OpParameter {
            name: param.name.clone(),
            rust_name: param.name.to_snake_case(),
            location,
            required,
            rust_type,
            description: param.description.clone(),
        }
    }

    fn convert_request_body(
        &mut self,
        body: Option<&spec::RequestBody>,
        operation_id: &str,
    ) -> Result<Option<OpRequestBody>> {
        let body = match body {
            Some(b) => {
                // Resolve $ref if present.
                if let Some(ref_path) = &b.ref_path {
                    match self.spec.resolve_request_body_ref(ref_path) {
                        Some(resolved) => resolved.clone(),
                        None => return Ok(None),
                    }
                } else {
                    b.clone()
                }
            }
            None => return Ok(None),
        };

        // Find a JSON media type.
        let schema = body
            .content
            .get("application/json")
            .or_else(|| body.content.get("*/*"))
            .and_then(|mt| mt.schema.as_ref());

        let schema = match schema {
            Some(s) => s,
            None => return Ok(None),
        };

        // Resolve the schema if it's a $ref.
        let (resolved_schema, type_name) = if let Some(ref_path) = &schema.ref_path {
            let name = spec::ref_name(ref_path);
            let resolved = self
                .spec
                .resolve_schema_ref(ref_path)
                .cloned()
                .unwrap_or_default();
            self.ensure_type(name, &resolved);
            (resolved, Some(name.to_upper_camel_case()))
        } else {
            (schema.clone(), None)
        };

        let fields = self.schema_to_fields(&self.merge_all_of(&resolved_schema));

        // If the body is an inline object with no name, create a type for it.
        let type_name = type_name.or_else(|| {
            if !fields.is_empty() {
                let name = format!("{operation_id}_body").to_upper_camel_case();
                let schema_copy = resolved_schema.clone();
                self.ensure_type(&name, &schema_copy);
                Some(name)
            } else {
                None
            }
        });

        Ok(Some(OpRequestBody {
            required: body.required,
            fields,
            type_name,
        }))
    }

    fn extract_response_type(
        &mut self,
        responses: &BTreeMap<std::string::String, spec::Response>,
    ) -> Option<RustType> {
        // Look for 200, 201, 2XX in order.
        let candidates = ["200", "201", "202", "204", "2XX", "default"];
        for code in &candidates {
            if let Some(resp) = responses.get(*code) {
                // Resolve $ref.
                let resp = if let Some(ref_path) = &resp.ref_path {
                    self.spec
                        .resolve_response_ref(ref_path)
                        .cloned()
                        .unwrap_or_else(|| resp.clone())
                } else {
                    resp.clone()
                };

                if let Some(content) = &resp.content {
                    let schema = content
                        .get("application/json")
                        .or_else(|| content.get("*/*"))
                        .and_then(|mt| mt.schema.as_ref());

                    if let Some(s) = schema {
                        return Some(self.schema_to_rust_type(s, None));
                    }
                }
            }
        }
        None
    }
}
