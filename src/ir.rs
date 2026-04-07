// Intermediate representation derived from parsed OpenAPI specs.

use std::collections::BTreeMap;

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
    /// Convert a parsed `OpenAPI` spec into the intermediate representation.
    #[must_use]
    pub fn from_openapi(spec: &OpenApiSpec) -> Self {
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

    fn run(&mut self) -> ApiSpec {
        self.convert_component_schemas();
        let operations = self.convert_operations();
        let auth = self.detect_auth();
        let base_url = self.spec.servers.first().map(|s| s.url.clone());

        ApiSpec {
            name: self.spec.info.title.clone(),
            description: self.spec.info.description.clone(),
            version: self.spec.info.version.clone(),
            base_url,
            auth,
            operations,
            types: self.types.clone(),
        }
    }

    // ── Auth detection ─────────────────────────────────────────────────

    fn detect_auth(&self) -> AuthMethod {
        let Some(components) = &self.spec.components else {
            return AuthMethod::None;
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
                    if scheme.location.as_deref() == Some("header")
                        && let Some(name) = &scheme.name
                    {
                        return AuthMethod::ApiKeyHeader(name.clone());
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
        let mut merged = Schema {
            schema_type: Some("object".into()),
            ..Schema::default()
        };

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

    /// Convert a schema's properties into a vec of `FieldDef`.
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

    /// Convert an `OpenAPI` Schema to a `RustType`, optionally creating named
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
                Some("uint64") => RustType::U64,
                _ => RustType::I64,
            },
            Some("number") => RustType::F64,
            Some("boolean") => RustType::Bool,
            Some("array") => {
                let inner = schema
                    .items
                    .as_ref()
                    .map_or(RustType::Value, |s| {
                        self.schema_to_rust_type(s, context_name)
                    });
                RustType::Vec(Box::new(inner))
            }
            Some("object") => {
                if schema.properties.is_empty() {
                    return RustType::Value;
                }
                if let Some(ctx) = context_name {
                    let type_name = ctx.to_upper_camel_case();
                    self.ensure_type(ctx, schema);
                    return RustType::Named(type_name);
                }
                RustType::Value
            }
            _ => {
                if !schema.properties.is_empty()
                    && let Some(ctx) = context_name
                {
                    let type_name = ctx.to_upper_camel_case();
                    self.ensure_type(ctx, schema);
                    return RustType::Named(type_name);
                }
                RustType::Value
            }
        }
    }

    // ── Operations ─────────────────────────────────────────────────────

    fn convert_operations(&mut self) -> Vec<Operation> {
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
                    ops.push(self.convert_operation(*method, path, op, path_params));
                }
            }
        }

        ops
    }

    fn convert_operation(
        &mut self,
        method: HttpMethod,
        path: &str,
        op: &spec::Operation,
        path_level_params: &[spec::Parameter],
    ) -> Operation {
        let id = op
            .operation_id
            .clone()
            .unwrap_or_else(|| {
                format!("{}_{}", format!("{method}").to_lowercase(), path.replace('/', "_"))
            })
            .to_snake_case();

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

        let request_body = self.convert_request_body(op.request_body.as_ref(), &id);
        let response_type = self.extract_response_type(&op.responses);

        let errors: Vec<ErrorResponse> = op
            .responses
            .iter()
            .filter(|(code, _)| !code.starts_with('2'))
            .map(|(code, resp)| ErrorResponse {
                status_code: code.clone(),
                description: resp.description.clone(),
            })
            .collect();

        Operation {
            id,
            method,
            path: path.to_string(),
            summary: op.summary.clone(),
            description: op.description.clone(),
            parameters,
            request_body,
            response_type,
            errors,
        }
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
            .map_or(RustType::String, |s| {
                self.schema_to_rust_type(s, Some(&param.name))
            });

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
    ) -> Option<OpRequestBody> {
        let b = body?;

        let body = if let Some(ref_path) = &b.ref_path {
            self.spec
                .resolve_request_body_ref(ref_path)
                .cloned()?
        } else {
            b.clone()
        };

        let schema = body
            .content
            .get("application/json")
            .or_else(|| body.content.get("*/*"))
            .and_then(|mt| mt.schema.as_ref())?;

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

        let type_name = type_name.or_else(|| {
            if fields.is_empty() {
                return None;
            }
            let name = format!("{operation_id}_body").to_upper_camel_case();
            let schema_copy = resolved_schema.clone();
            self.ensure_type(&name, &schema_copy);
            Some(name)
        });

        Some(OpRequestBody {
            required: body.required,
            fields,
            type_name,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::OpenApiSpec;

    /// Shared helper: parse a YAML spec and convert to IR.
    fn parse_ir(yaml: &str) -> ApiSpec {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(yaml).unwrap();
        ApiSpec::from_openapi(&spec)
    }

    const PETSTORE_YAML: &str = r##"
info:
  title: Pet Store
  description: A sample pet store API
  version: "2.0.0"
servers:
  - url: https://api.petstore.example.com/v2
paths:
  /pets:
    get:
      operationId: listPets
      summary: List all pets
      parameters:
        - name: limit
          in: query
          required: false
          schema:
            type: integer
            format: int64
      responses:
        "200":
          description: A list of pets
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Pet"
    post:
      operationId: createPet
      summary: Create a pet
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreatePetRequest"
      responses:
        "201":
          description: Pet created
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Pet"
  /pets/{petId}:
    parameters:
      - name: petId
        in: path
        required: true
        schema:
          type: string
    get:
      operationId: getPet
      summary: Get a pet by ID
      responses:
        "200":
          description: A pet
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Pet"
        "404":
          description: Pet not found
    delete:
      operationId: deletePet
      summary: Delete a pet
      responses:
        "204":
          description: Pet deleted
components:
  schemas:
    Pet:
      type: object
      required:
        - id
        - name
      properties:
        id:
          type: integer
          format: int64
        name:
          type: string
        tag:
          type: string
        status:
          $ref: "#/components/schemas/PetStatus"
    PetStatus:
      type: string
      enum:
        - available
        - pending
        - sold
    CreatePetRequest:
      type: object
      required:
        - name
      properties:
        name:
          type: string
          description: The pet's name
        tag:
          type: string
          description: Optional tag
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
"##;

    // -- Top-level ApiSpec conversion --

    #[test]
    fn api_spec_name_and_version() {
        let api = parse_ir(PETSTORE_YAML);
        assert_eq!(api.name, "Pet Store");
        assert_eq!(api.version, "2.0.0");
        assert_eq!(
            api.description.as_deref(),
            Some("A sample pet store API")
        );
    }

    #[test]
    fn api_spec_base_url() {
        let api = parse_ir(PETSTORE_YAML);
        assert_eq!(
            api.base_url.as_deref(),
            Some("https://api.petstore.example.com/v2")
        );
    }

    #[test]
    fn api_spec_no_servers_yields_none_base_url() {
        let yaml = r#"
info:
  title: NoServer
  version: "1.0.0"
paths: {}
"#;
        let api = parse_ir(yaml);
        assert!(api.base_url.is_none());
    }

    // -- Auth detection --

    #[test]
    fn detect_bearer_auth() {
        let api = parse_ir(PETSTORE_YAML);
        assert_eq!(api.auth, AuthMethod::Bearer);
    }

    #[test]
    fn detect_basic_auth() {
        let yaml = r#"
info:
  title: Basic Auth
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    basicAuth:
      type: http
      scheme: basic
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::Basic);
    }

    #[test]
    fn detect_api_key_header_auth() {
        let yaml = r#"
info:
  title: ApiKey Auth
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    apiKey:
      type: apiKey
      in: header
      name: X-API-Key
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::ApiKeyHeader("X-API-Key".into()));
    }

    #[test]
    fn detect_no_auth() {
        let yaml = r#"
info:
  title: No Auth
  version: "1.0.0"
paths: {}
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::None);
    }

    // -- Type conversion --

    #[test]
    fn types_include_all_component_schemas() {
        let api = parse_ir(PETSTORE_YAML);
        let type_names: Vec<&str> = api.types.iter().map(|t| t.rust_name.as_str()).collect();
        assert!(type_names.contains(&"Pet"));
        assert!(type_names.contains(&"PetStatus"));
        assert!(type_names.contains(&"CreatePetRequest"));
    }

    #[test]
    fn struct_type_has_correct_fields() {
        let api = parse_ir(PETSTORE_YAML);
        let pet = api.types.iter().find(|t| t.rust_name == "Pet").unwrap();
        assert!(!pet.is_enum);
        assert!(pet.enum_variants.is_empty());

        let field_names: Vec<&str> = pet.fields.iter().map(|f| f.rust_name.as_str()).collect();
        assert!(field_names.contains(&"id"));
        assert!(field_names.contains(&"name"));
        assert!(field_names.contains(&"tag"));
        assert!(field_names.contains(&"status"));
    }

    #[test]
    fn required_fields_are_marked() {
        let api = parse_ir(PETSTORE_YAML);
        let pet = api.types.iter().find(|t| t.rust_name == "Pet").unwrap();

        let id_field = pet.fields.iter().find(|f| f.rust_name == "id").unwrap();
        assert!(id_field.required);
        assert_eq!(id_field.rust_type, RustType::I64);

        let name_field = pet.fields.iter().find(|f| f.rust_name == "name").unwrap();
        assert!(name_field.required);
        assert_eq!(name_field.rust_type, RustType::String);
    }

    #[test]
    fn optional_fields_get_option_type() {
        let api = parse_ir(PETSTORE_YAML);
        let pet = api.types.iter().find(|t| t.rust_name == "Pet").unwrap();

        let tag_field = pet.fields.iter().find(|f| f.rust_name == "tag").unwrap();
        assert!(!tag_field.required);
        assert_eq!(tag_field.rust_type, RustType::Option(Box::new(RustType::String)));
    }

    #[test]
    fn enum_type_is_detected() {
        let api = parse_ir(PETSTORE_YAML);
        let status = api.types.iter().find(|t| t.rust_name == "PetStatus").unwrap();
        assert!(status.is_enum);
        assert!(status.fields.is_empty());
        assert_eq!(status.enum_variants.len(), 3);

        let variant_names: Vec<&str> =
            status.enum_variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(variant_names, vec!["available", "pending", "sold"]);
    }

    #[test]
    fn enum_variant_rust_names() {
        let api = parse_ir(PETSTORE_YAML);
        let status = api.types.iter().find(|t| t.rust_name == "PetStatus").unwrap();
        let rust_names: Vec<&str> =
            status.enum_variants.iter().map(|v| v.rust_name.as_str()).collect();
        assert_eq!(rust_names, vec!["Available", "Pending", "Sold"]);
    }

    #[test]
    fn ref_field_resolves_to_named_type() {
        let api = parse_ir(PETSTORE_YAML);
        let pet = api.types.iter().find(|t| t.rust_name == "Pet").unwrap();
        let status_field = pet.fields.iter().find(|f| f.rust_name == "status").unwrap();
        // status is optional (not in required) and references PetStatus
        assert_eq!(
            status_field.rust_type,
            RustType::Option(Box::new(RustType::Named("PetStatus".into())))
        );
    }

    // -- Operations --

    #[test]
    fn operations_count() {
        let api = parse_ir(PETSTORE_YAML);
        // listPets, createPet, getPet, deletePet
        assert_eq!(api.operations.len(), 4);
    }

    #[test]
    fn operation_ids_are_snake_cased() {
        let api = parse_ir(PETSTORE_YAML);
        let ids: Vec<&str> = api.operations.iter().map(|o| o.id.as_str()).collect();
        assert!(ids.contains(&"list_pets"));
        assert!(ids.contains(&"create_pet"));
        assert!(ids.contains(&"get_pet"));
        assert!(ids.contains(&"delete_pet"));
    }

    #[test]
    fn operation_methods() {
        let api = parse_ir(PETSTORE_YAML);
        let list = api.operations.iter().find(|o| o.id == "list_pets").unwrap();
        assert_eq!(list.method, HttpMethod::Get);
        let create = api.operations.iter().find(|o| o.id == "create_pet").unwrap();
        assert_eq!(create.method, HttpMethod::Post);
        let delete = api.operations.iter().find(|o| o.id == "delete_pet").unwrap();
        assert_eq!(delete.method, HttpMethod::Delete);
    }

    #[test]
    fn operation_paths() {
        let api = parse_ir(PETSTORE_YAML);
        let list = api.operations.iter().find(|o| o.id == "list_pets").unwrap();
        assert_eq!(list.path, "/pets");
        let get = api.operations.iter().find(|o| o.id == "get_pet").unwrap();
        assert_eq!(get.path, "/pets/{petId}");
    }

    #[test]
    fn operation_summary() {
        let api = parse_ir(PETSTORE_YAML);
        let list = api.operations.iter().find(|o| o.id == "list_pets").unwrap();
        assert_eq!(list.summary.as_deref(), Some("List all pets"));
    }

    #[test]
    fn operation_query_parameter() {
        let api = parse_ir(PETSTORE_YAML);
        let list = api.operations.iter().find(|o| o.id == "list_pets").unwrap();
        assert_eq!(list.parameters.len(), 1);
        let limit = &list.parameters[0];
        assert_eq!(limit.name, "limit");
        assert_eq!(limit.rust_name, "limit");
        assert_eq!(limit.location, ParamLocation::Query);
        assert!(!limit.required);
        // Not required, so wrapped in Option
        assert_eq!(limit.rust_type, RustType::Option(Box::new(RustType::I64)));
    }

    #[test]
    fn operation_path_parameter_from_path_level() {
        let api = parse_ir(PETSTORE_YAML);
        let get = api.operations.iter().find(|o| o.id == "get_pet").unwrap();
        let pet_id = get.parameters.iter().find(|p| p.name == "petId").unwrap();
        assert_eq!(pet_id.location, ParamLocation::Path);
        assert!(pet_id.required);
        assert_eq!(pet_id.rust_type, RustType::String);
    }

    #[test]
    fn operation_request_body() {
        let api = parse_ir(PETSTORE_YAML);
        let create = api.operations.iter().find(|o| o.id == "create_pet").unwrap();
        let body = create.request_body.as_ref().unwrap();
        assert!(body.required);
        assert_eq!(body.type_name.as_deref(), Some("CreatePetRequest"));
        assert_eq!(body.fields.len(), 2);
    }

    #[test]
    fn operation_no_request_body() {
        let api = parse_ir(PETSTORE_YAML);
        let list = api.operations.iter().find(|o| o.id == "list_pets").unwrap();
        assert!(list.request_body.is_none());
    }

    #[test]
    fn operation_response_type() {
        let api = parse_ir(PETSTORE_YAML);
        let get = api.operations.iter().find(|o| o.id == "get_pet").unwrap();
        assert_eq!(
            get.response_type,
            Some(RustType::Named("Pet".into()))
        );
    }

    #[test]
    fn operation_array_response_type() {
        let api = parse_ir(PETSTORE_YAML);
        let list = api.operations.iter().find(|o| o.id == "list_pets").unwrap();
        assert_eq!(
            list.response_type,
            Some(RustType::Vec(Box::new(RustType::Named("Pet".into()))))
        );
    }

    #[test]
    fn operation_no_response_type_for_204() {
        let api = parse_ir(PETSTORE_YAML);
        let delete = api.operations.iter().find(|o| o.id == "delete_pet").unwrap();
        // 204 has no content, so response_type should be None
        assert!(delete.response_type.is_none());
    }

    #[test]
    fn operation_error_responses() {
        let api = parse_ir(PETSTORE_YAML);
        let get = api.operations.iter().find(|o| o.id == "get_pet").unwrap();
        assert_eq!(get.errors.len(), 1);
        assert_eq!(get.errors[0].status_code, "404");
        assert_eq!(
            get.errors[0].description.as_deref(),
            Some("Pet not found")
        );
    }

    // -- HttpMethod Display --

    #[test]
    fn http_method_display() {
        assert_eq!(format!("{}", HttpMethod::Get), "GET");
        assert_eq!(format!("{}", HttpMethod::Post), "POST");
        assert_eq!(format!("{}", HttpMethod::Put), "PUT");
        assert_eq!(format!("{}", HttpMethod::Delete), "DELETE");
        assert_eq!(format!("{}", HttpMethod::Patch), "PATCH");
    }

    // -- RustType Display --

    #[test]
    fn rust_type_display_primitives() {
        assert_eq!(format!("{}", RustType::String), "String");
        assert_eq!(format!("{}", RustType::I64), "i64");
        assert_eq!(format!("{}", RustType::U64), "u64");
        assert_eq!(format!("{}", RustType::F64), "f64");
        assert_eq!(format!("{}", RustType::Bool), "bool");
        assert_eq!(format!("{}", RustType::Value), "serde_json::Value");
    }

    #[test]
    fn rust_type_display_vec() {
        let vec_type = RustType::Vec(Box::new(RustType::String));
        assert_eq!(format!("{vec_type}"), "Vec<String>");
    }

    #[test]
    fn rust_type_display_option() {
        let opt_type = RustType::Option(Box::new(RustType::I64));
        assert_eq!(format!("{opt_type}"), "Option<i64>");
    }

    #[test]
    fn rust_type_display_named() {
        let named = RustType::Named("Pet".into());
        assert_eq!(format!("{named}"), "Pet");
    }

    #[test]
    fn rust_type_display_nested() {
        let nested = RustType::Option(Box::new(RustType::Vec(Box::new(RustType::Named(
            "Pet".into(),
        )))));
        assert_eq!(format!("{nested}"), "Option<Vec<Pet>>");
    }

    // -- Schema type mapping --

    #[test]
    fn schema_type_integer_default_is_i64() {
        let yaml = r#"
info:
  title: IntegerTest
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      parameters:
        - name: count
          in: query
          required: true
          schema:
            type: integer
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let op = &api.operations[0];
        let param = &op.parameters[0];
        assert_eq!(param.rust_type, RustType::I64);
    }

    #[test]
    fn schema_type_number_is_f64() {
        let yaml = r#"
info:
  title: NumberTest
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      parameters:
        - name: amount
          in: query
          required: true
          schema:
            type: number
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let param = &api.operations[0].parameters[0];
        assert_eq!(param.rust_type, RustType::F64);
    }

    #[test]
    fn schema_type_boolean() {
        let yaml = r#"
info:
  title: BoolTest
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      parameters:
        - name: active
          in: query
          required: true
          schema:
            type: boolean
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let param = &api.operations[0].parameters[0];
        assert_eq!(param.rust_type, RustType::Bool);
    }

    // -- allOf merging --

    #[test]
    fn all_of_merges_properties() {
        let yaml = r##"
info:
  title: AllOf Test
  version: "1.0.0"
paths: {}
components:
  schemas:
    Base:
      type: object
      required:
        - id
      properties:
        id:
          type: integer
    Extended:
      allOf:
        - $ref: "#/components/schemas/Base"
        - type: object
          required:
            - extra
          properties:
            extra:
              type: string
"##;
        let api = parse_ir(yaml);
        let extended = api.types.iter().find(|t| t.rust_name == "Extended").unwrap();
        let field_names: Vec<&str> =
            extended.fields.iter().map(|f| f.rust_name.as_str()).collect();
        assert!(field_names.contains(&"id"));
        assert!(field_names.contains(&"extra"));

        let id = extended.fields.iter().find(|f| f.rust_name == "id").unwrap();
        assert!(id.required);
        let extra = extended.fields.iter().find(|f| f.rust_name == "extra").unwrap();
        assert!(extra.required);
    }

    // -- Generated operation ID fallback --

    #[test]
    fn operation_without_operation_id_gets_generated_id() {
        let yaml = r#"
info:
  title: NoId
  version: "1.0.0"
paths:
  /items:
    get:
      summary: List items
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.operations.len(), 1);
        // Should be generated from method + path
        let op = &api.operations[0];
        assert!(!op.id.is_empty());
        // The pattern is "get__items" from "get_/items"
        assert!(op.id.contains("get"), "generated id should contain method");
    }

    // -- Inline object in request body --

    #[test]
    fn inline_request_body_creates_type() {
        let yaml = r#"
info:
  title: InlineBody
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required:
                - name
              properties:
                name:
                  type: string
                count:
                  type: integer
      responses:
        "201":
          description: created
"#;
        let api = parse_ir(yaml);
        let create = api.operations.iter().find(|o| o.id == "create_item").unwrap();
        let body = create.request_body.as_ref().unwrap();
        // An inline body should get a generated type name
        assert!(body.type_name.is_some());
        assert_eq!(body.fields.len(), 2);
    }

    // -- No duplicate types --

    #[test]
    fn no_duplicate_type_definitions() {
        let api = parse_ir(PETSTORE_YAML);
        let mut names: Vec<&str> = api.types.iter().map(|t| t.rust_name.as_str()).collect();
        let len_before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), len_before, "duplicate type definitions found");
    }

    // -- Field descriptions --

    #[test]
    fn field_descriptions_are_preserved() {
        let api = parse_ir(PETSTORE_YAML);
        let create_req = api
            .types
            .iter()
            .find(|t| t.rust_name == "CreatePetRequest")
            .unwrap();
        let name_field = create_req
            .fields
            .iter()
            .find(|f| f.rust_name == "name")
            .unwrap();
        assert_eq!(name_field.description.as_deref(), Some("The pet's name"));
    }

    // -- Integer format uint64 --

    #[test]
    fn schema_type_integer_uint64() {
        let yaml = r#"
info:
  title: Uint64Test
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      parameters:
        - name: big_count
          in: query
          required: true
          schema:
            type: integer
            format: uint64
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let param = &api.operations[0].parameters[0];
        assert_eq!(param.rust_type, RustType::U64);
    }

    // -- Header parameter location --

    #[test]
    fn header_parameter_location() {
        let yaml = r#"
info:
  title: HeaderTest
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      parameters:
        - name: X-Request-Id
          in: header
          required: false
          schema:
            type: string
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let param = &api.operations[0].parameters[0];
        assert_eq!(param.location, ParamLocation::Header);
        assert!(!param.required);
        assert_eq!(param.rust_type, RustType::Option(Box::new(RustType::String)));
    }

    // -- oneOf / anyOf fallback to Value --

    #[test]
    fn one_of_resolves_to_value() {
        let yaml = r##"
info:
  title: OneOfTest
  version: "1.0.0"
paths: {}
components:
  schemas:
    Mixed:
      type: object
      properties:
        data:
          oneOf:
            - type: string
            - type: integer
      required:
        - data
"##;
        let api = parse_ir(yaml);
        let mixed = api.types.iter().find(|t| t.rust_name == "Mixed").unwrap();
        let data = mixed.fields.iter().find(|f| f.rust_name == "data").unwrap();
        assert_eq!(data.rust_type, RustType::Value);
    }

    #[test]
    fn any_of_resolves_to_value() {
        let yaml = r##"
info:
  title: AnyOfTest
  version: "1.0.0"
paths: {}
components:
  schemas:
    Flexible:
      type: object
      properties:
        payload:
          anyOf:
            - type: string
            - type: number
      required:
        - payload
"##;
        let api = parse_ir(yaml);
        let flex = api.types.iter().find(|t| t.rust_name == "Flexible").unwrap();
        let payload = flex.fields.iter().find(|f| f.rust_name == "payload").unwrap();
        assert_eq!(payload.rust_type, RustType::Value);
    }

    // -- Array with no items schema --

    #[test]
    fn array_with_no_items_uses_value() {
        let yaml = r##"
info:
  title: ArrayNoItems
  version: "1.0.0"
paths: {}
components:
  schemas:
    Container:
      type: object
      required:
        - things
      properties:
        things:
          type: array
"##;
        let api = parse_ir(yaml);
        let container = api.types.iter().find(|t| t.rust_name == "Container").unwrap();
        let things = container.fields.iter().find(|f| f.rust_name == "things").unwrap();
        assert_eq!(things.rust_type, RustType::Vec(Box::new(RustType::Value)));
    }

    // -- Inline enum at field level --

    #[test]
    fn inline_enum_creates_named_type() {
        let yaml = r##"
info:
  title: InlineEnum
  version: "1.0.0"
paths: {}
components:
  schemas:
    Task:
      type: object
      required:
        - priority
      properties:
        priority:
          type: string
          enum:
            - low
            - medium
            - high
"##;
        let api = parse_ir(yaml);
        let task = api.types.iter().find(|t| t.rust_name == "Task").unwrap();
        let prio_field = task.fields.iter().find(|f| f.rust_name == "priority").unwrap();
        assert!(matches!(prio_field.rust_type, RustType::Named(_)));

        let prio_type = api.types.iter().find(|t| t.rust_name == "Priority").unwrap();
        assert!(prio_type.is_enum);
        assert_eq!(prio_type.enum_variants.len(), 3);
    }

    // -- Object with additional_properties falls back to Value --

    #[test]
    fn object_with_additional_properties_is_value() {
        let yaml = r##"
info:
  title: AdditionalProps
  version: "1.0.0"
paths: {}
components:
  schemas:
    Metadata:
      type: object
      additionalProperties:
        type: string
"##;
        let api = parse_ir(yaml);
        let meta = api.types.iter().find(|t| t.rust_name == "Metadata").unwrap();
        assert!(meta.fields.is_empty());
    }

    // -- No explicit type but has properties (implicit object) --

    #[test]
    fn implicit_object_with_properties() {
        let yaml = r##"
info:
  title: ImplicitObj
  version: "1.0.0"
paths: {}
components:
  schemas:
    Implicit:
      properties:
        name:
          type: string
      required:
        - name
"##;
        let api = parse_ir(yaml);
        let implicit = api.types.iter().find(|t| t.rust_name == "Implicit").unwrap();
        assert!(!implicit.fields.is_empty());
        let name_field = implicit.fields.iter().find(|f| f.rust_name == "name").unwrap();
        assert_eq!(name_field.rust_type, RustType::String);
    }

    // -- apiKey in non-header location is ignored --

    #[test]
    fn api_key_non_header_auth_is_none() {
        let yaml = r#"
info:
  title: ApiKeyQuery
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    queryKey:
      type: apiKey
      in: query
      name: api_key
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::None);
    }

    // -- apiKey header with missing name --

    #[test]
    fn api_key_header_no_name_is_none() {
        let yaml = r#"
info:
  title: ApiKeyNoName
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    headerKey:
      type: apiKey
      in: header
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::None);
    }

    // -- Unknown security scheme type --

    #[test]
    fn unknown_security_scheme_type_is_none() {
        let yaml = r#"
info:
  title: UnknownScheme
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    oauth:
      type: oauth2
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::None);
    }

    // -- HTTP scheme with unrecognized scheme name --

    #[test]
    fn http_scheme_unknown_scheme_is_none() {
        let yaml = r#"
info:
  title: UnknownHttpScheme
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    digest:
      type: http
      scheme: digest
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::None);
    }

    // -- Operation parameter override (op-level overrides path-level) --

    #[test]
    fn operation_param_overrides_path_param() {
        let yaml = r#"
info:
  title: ParamOverride
  version: "1.0.0"
paths:
  /items:
    parameters:
      - name: page
        in: query
        required: false
        schema:
          type: integer
    get:
      operationId: listItems
      parameters:
        - name: page
          in: query
          required: true
          schema:
            type: integer
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "list_items").unwrap();
        let page = op.parameters.iter().find(|p| p.name == "page").unwrap();
        assert!(page.required, "op-level param should override path-level required=false");
    }

    // -- Multiple 2xx responses: picks first in priority order --

    #[test]
    fn response_type_priority_200_over_201() {
        let yaml = r##"
info:
  title: MultiResponse
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: createItem
      responses:
        "201":
          description: Created
          content:
            application/json:
              schema:
                type: object
                properties:
                  created_id:
                    type: string
                required:
                  - created_id
        "200":
          description: Already existed
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Item"
components:
  schemas:
    Item:
      type: object
      properties:
        id:
          type: integer
"##;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "create_item").unwrap();
        assert_eq!(op.response_type, Some(RustType::Named("Item".into())));
    }

    // -- Request body with */* media type --

    #[test]
    fn request_body_wildcard_media_type() {
        let yaml = r#"
info:
  title: WildcardMedia
  version: "1.0.0"
paths:
  /items:
    post:
      operationId: createItem
      requestBody:
        required: true
        content:
          "*/*":
            schema:
              type: object
              properties:
                data:
                  type: string
              required:
                - data
      responses:
        "201":
          description: created
"#;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "create_item").unwrap();
        let body = op.request_body.as_ref().unwrap();
        assert!(!body.fields.is_empty());
    }

    // -- Request body with no recognized media type --

    #[test]
    fn request_body_unrecognized_media_type_returns_none() {
        let yaml = r#"
info:
  title: NoJsonBody
  version: "1.0.0"
paths:
  /upload:
    post:
      operationId: uploadFile
      requestBody:
        required: true
        content:
          multipart/form-data:
            schema:
              type: object
              properties:
                file:
                  type: string
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "upload_file").unwrap();
        assert!(op.request_body.is_none());
    }

    // -- Path parameter is always required --

    #[test]
    fn path_params_are_always_required() {
        let yaml = r#"
info:
  title: PathRequired
  version: "1.0.0"
paths:
  /items/{id}:
    get:
      operationId: getItem
      parameters:
        - name: id
          in: path
          required: false
          schema:
            type: string
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "get_item").unwrap();
        let id_param = op.parameters.iter().find(|p| p.name == "id").unwrap();
        assert!(id_param.required, "path params must always be required even if spec says false");
        assert_eq!(id_param.rust_type, RustType::String);
    }

    // -- PATCH method --

    #[test]
    fn patch_method_is_detected() {
        let yaml = r#"
info:
  title: PatchTest
  version: "1.0.0"
paths:
  /items/{id}:
    patch:
      operationId: patchItem
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "patch_item").unwrap();
        assert_eq!(op.method, HttpMethod::Patch);
    }

    // -- No content in 204 response --

    #[test]
    fn no_response_type_from_empty_204() {
        let yaml = r#"
info:
  title: Empty204
  version: "1.0.0"
paths:
  /items/{id}:
    delete:
      operationId: deleteItem
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        "204":
          description: No content
"#;
        let api = parse_ir(yaml);
        let op = api.operations.iter().find(|o| o.id == "delete_item").unwrap();
        assert!(op.response_type.is_none());
    }

    // -- allOf with direct properties on parent schema --

    #[test]
    fn all_of_with_parent_properties() {
        let yaml = r##"
info:
  title: AllOfParent
  version: "1.0.0"
paths: {}
components:
  schemas:
    Base:
      type: object
      properties:
        id:
          type: integer
      required:
        - id
    Child:
      allOf:
        - $ref: "#/components/schemas/Base"
      properties:
        name:
          type: string
      required:
        - name
"##;
        let api = parse_ir(yaml);
        let child = api.types.iter().find(|t| t.rust_name == "Child").unwrap();
        let field_names: Vec<&str> = child.fields.iter().map(|f| f.rust_name.as_str()).collect();
        assert!(field_names.contains(&"id"), "should inherit base fields");
        assert!(field_names.contains(&"name"), "should have own fields");
    }

    // -- Multiple operations on the same path --

    #[test]
    fn multiple_methods_on_same_path() {
        let yaml = r#"
info:
  title: MultiMethod
  version: "1.0.0"
paths:
  /items:
    get:
      operationId: listItems
      responses:
        "200":
          description: ok
    post:
      operationId: createItem
      responses:
        "201":
          description: created
    put:
      operationId: replaceItems
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.operations.len(), 3);
        let methods: Vec<HttpMethod> = api.operations.iter().map(|o| o.method).collect();
        assert!(methods.contains(&HttpMethod::Get));
        assert!(methods.contains(&HttpMethod::Post));
        assert!(methods.contains(&HttpMethod::Put));
    }

    // -- Empty components section --

    #[test]
    fn empty_paths_produces_no_operations() {
        let yaml = r#"
info:
  title: EmptyPaths
  version: "1.0.0"
paths: {}
"#;
        let api = parse_ir(yaml);
        assert!(api.operations.is_empty());
        assert!(api.types.is_empty());
    }

    // -- Default values on fields --

    #[test]
    fn field_default_value_is_preserved() {
        let yaml = r##"
info:
  title: DefaultVal
  version: "1.0.0"
paths: {}
components:
  schemas:
    Config:
      type: object
      properties:
        retries:
          type: integer
          default: 3
        mode:
          type: string
          default: "auto"
"##;
        let api = parse_ir(yaml);
        let config = api.types.iter().find(|t| t.rust_name == "Config").unwrap();
        let retries = config.fields.iter().find(|f| f.rust_name == "retries").unwrap();
        assert_eq!(retries.default_value, Some(serde_json::json!(3)));
        let mode = config.fields.iter().find(|f| f.rust_name == "mode").unwrap();
        assert_eq!(mode.default_value, Some(serde_json::json!("auto")));
    }

    // -- Operation with description (not summary) --

    #[test]
    fn operation_description_is_preserved() {
        let yaml = r#"
info:
  title: DescTest
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      description: A longer description of the test operation
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let op = &api.operations[0];
        assert_eq!(
            op.description.as_deref(),
            Some("A longer description of the test operation")
        );
    }

    // -- Parameter without schema defaults to String --

    #[test]
    fn parameter_without_schema_defaults_to_string() {
        let yaml = r#"
info:
  title: NoSchemaParam
  version: "1.0.0"
paths:
  /test:
    get:
      operationId: test
      parameters:
        - name: token
          in: query
          required: true
      responses:
        "200":
          description: ok
"#;
        let api = parse_ir(yaml);
        let param = &api.operations[0].parameters[0];
        assert_eq!(param.rust_type, RustType::String);
    }

    // -- Nested allOf with context_name generates Named type --

    #[test]
    fn all_of_at_field_level_creates_named_type() {
        let yaml = r##"
info:
  title: FieldAllOf
  version: "1.0.0"
paths: {}
components:
  schemas:
    Base:
      type: object
      properties:
        id:
          type: integer
    Wrapper:
      type: object
      required:
        - nested
      properties:
        nested:
          allOf:
            - $ref: "#/components/schemas/Base"
            - type: object
              properties:
                extra:
                  type: string
"##;
        let api = parse_ir(yaml);
        let wrapper = api.types.iter().find(|t| t.rust_name == "Wrapper").unwrap();
        let nested = wrapper.fields.iter().find(|f| f.rust_name == "nested").unwrap();
        assert!(matches!(nested.rust_type, RustType::Named(_)));
    }

    // -- Name (title) is preserved exactly (not snake-cased) --

    #[test]
    fn api_name_is_raw_title() {
        let yaml = r#"
info:
  title: My Awesome API
  version: "1.0.0"
paths: {}
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.name, "My Awesome API");
    }

    // -- Bearer auth case insensitive --

    #[test]
    fn bearer_auth_case_insensitive() {
        let yaml = r#"
info:
  title: CaseAuth
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    auth:
      type: http
      scheme: Bearer
"#;
        let api = parse_ir(yaml);
        assert_eq!(api.auth, AuthMethod::Bearer);
    }
}
