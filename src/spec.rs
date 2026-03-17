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

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal OpenAPI 3.0.3 spec in YAML for testing.
    const MINIMAL_SPEC_YAML: &str = r#"
info:
  title: Test API
  version: "1.0.0"
paths: {}
"#;

    /// A richer spec with components, paths, servers, and security schemes.
    const FULL_SPEC_YAML: &str = r##"
info:
  title: Pet Store
  description: A sample pet store API
  version: "2.0.0"
servers:
  - url: https://api.petstore.example.com/v2
    description: Production server
security:
  - bearerAuth: []
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
  parameters:
    LimitParam:
      name: limit
      in: query
      required: false
      schema:
        type: integer
  requestBodies:
    PetBody:
      required: true
      content:
        application/json:
          schema:
            $ref: "#/components/schemas/CreatePetRequest"
  responses:
    NotFound:
      description: The requested resource was not found
"##;

    #[test]
    fn parse_minimal_yaml() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(MINIMAL_SPEC_YAML).unwrap();
        assert_eq!(spec.info.title, "Test API");
        assert_eq!(spec.info.version, "1.0.0");
        assert!(spec.paths.is_empty());
        assert!(spec.components.is_none());
        assert!(spec.servers.is_empty());
    }

    #[test]
    fn parse_full_yaml() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        assert_eq!(spec.info.title, "Pet Store");
        assert_eq!(
            spec.info.description.as_deref(),
            Some("A sample pet store API")
        );
        assert_eq!(spec.info.version, "2.0.0");
        assert_eq!(spec.servers.len(), 1);
        assert_eq!(spec.servers[0].url, "https://api.petstore.example.com/v2");
        assert_eq!(spec.paths.len(), 2);
        assert!(spec.paths.contains_key("/pets"));
        assert!(spec.paths.contains_key("/pets/{petId}"));
    }

    #[test]
    fn parse_path_item_methods() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let pets = &spec.paths["/pets"];
        assert!(pets.get.is_some());
        assert!(pets.post.is_some());
        assert!(pets.put.is_none());
        assert!(pets.delete.is_none());

        let pet_by_id = &spec.paths["/pets/{petId}"];
        assert!(pet_by_id.get.is_some());
        assert!(pet_by_id.delete.is_some());
        assert!(pet_by_id.post.is_none());
    }

    #[test]
    fn parse_operation_fields() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let list_op = spec.paths["/pets"].get.as_ref().unwrap();
        assert_eq!(list_op.operation_id.as_deref(), Some("listPets"));
        assert_eq!(list_op.summary.as_deref(), Some("List all pets"));
        assert_eq!(list_op.parameters.len(), 1);
        assert_eq!(list_op.parameters[0].name, "limit");
        assert_eq!(list_op.parameters[0].location, "query");
        assert!(!list_op.parameters[0].required);
    }

    #[test]
    fn parse_parameters_with_schema() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let limit_param = &spec.paths["/pets"].get.as_ref().unwrap().parameters[0];
        let schema = limit_param.schema.as_ref().unwrap();
        assert_eq!(schema.schema_type.as_deref(), Some("integer"));
        assert_eq!(schema.format.as_deref(), Some("int64"));
    }

    #[test]
    fn parse_path_level_parameters() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let pet_item = &spec.paths["/pets/{petId}"];
        assert_eq!(pet_item.parameters.len(), 1);
        assert_eq!(pet_item.parameters[0].name, "petId");
        assert_eq!(pet_item.parameters[0].location, "path");
        assert!(pet_item.parameters[0].required);
    }

    #[test]
    fn parse_request_body() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let create_op = spec.paths["/pets"].post.as_ref().unwrap();
        let body = create_op.request_body.as_ref().unwrap();
        assert!(body.required);
        assert!(body.content.contains_key("application/json"));
        let schema = body.content["application/json"].schema.as_ref().unwrap();
        assert_eq!(
            schema.ref_path.as_deref(),
            Some("#/components/schemas/CreatePetRequest")
        );
    }

    #[test]
    fn parse_responses() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let get_op = spec.paths["/pets/{petId}"].get.as_ref().unwrap();
        assert!(get_op.responses.contains_key("200"));
        assert!(get_op.responses.contains_key("404"));
        assert_eq!(
            get_op.responses["404"].description.as_deref(),
            Some("Pet not found")
        );
    }

    #[test]
    fn parse_component_schemas() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let components = spec.components.as_ref().unwrap();
        assert!(components.schemas.contains_key("Pet"));
        assert!(components.schemas.contains_key("PetStatus"));
        assert!(components.schemas.contains_key("CreatePetRequest"));

        let pet = &components.schemas["Pet"];
        assert_eq!(pet.schema_type.as_deref(), Some("object"));
        assert_eq!(pet.required, vec!["id", "name"]);
        assert!(pet.properties.contains_key("id"));
        assert!(pet.properties.contains_key("name"));
        assert!(pet.properties.contains_key("tag"));
        assert!(pet.properties.contains_key("status"));
    }

    #[test]
    fn parse_enum_schema() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let status = &spec.components.as_ref().unwrap().schemas["PetStatus"];
        assert_eq!(status.schema_type.as_deref(), Some("string"));
        let variants = status.enum_values.as_ref().unwrap();
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].as_str(), Some("available"));
        assert_eq!(variants[1].as_str(), Some("pending"));
        assert_eq!(variants[2].as_str(), Some("sold"));
    }

    #[test]
    fn parse_security_schemes() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let schemes = &spec.components.as_ref().unwrap().security_schemes;
        assert!(schemes.contains_key("bearerAuth"));
        let bearer = &schemes["bearerAuth"];
        assert_eq!(bearer.scheme_type, "http");
        assert_eq!(bearer.scheme.as_deref(), Some("bearer"));
    }

    #[test]
    fn parse_json_format() {
        let json = r#"{
            "info": { "title": "JSON API", "version": "0.1.0" },
            "paths": {}
        }"#;
        let spec: OpenApiSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.info.title, "JSON API");
        assert_eq!(spec.info.version, "0.1.0");
    }

    // -- ref_name tests --

    #[test]
    fn ref_name_extracts_last_segment() {
        assert_eq!(ref_name("#/components/schemas/Pet"), "Pet");
        assert_eq!(
            ref_name("#/components/parameters/LimitParam"),
            "LimitParam"
        );
        assert_eq!(ref_name("#/components/requestBodies/PetBody"), "PetBody");
    }

    #[test]
    fn ref_name_handles_no_slash() {
        assert_eq!(ref_name("Standalone"), "Standalone");
    }

    #[test]
    fn ref_name_handles_trailing_slash() {
        assert_eq!(ref_name("#/components/schemas/"), "");
    }

    // -- resolve_*_ref tests --

    #[test]
    fn resolve_schema_ref_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let pet = spec.resolve_schema_ref("#/components/schemas/Pet");
        assert!(pet.is_some());
        assert_eq!(pet.unwrap().schema_type.as_deref(), Some("object"));
    }

    #[test]
    fn resolve_schema_ref_not_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        assert!(
            spec.resolve_schema_ref("#/components/schemas/Nonexistent")
                .is_none()
        );
    }

    #[test]
    fn resolve_schema_ref_no_components() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(MINIMAL_SPEC_YAML).unwrap();
        assert!(
            spec.resolve_schema_ref("#/components/schemas/Anything")
                .is_none()
        );
    }

    #[test]
    fn resolve_parameter_ref_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let param = spec.resolve_parameter_ref("#/components/parameters/LimitParam");
        assert!(param.is_some());
        assert_eq!(param.unwrap().name, "limit");
    }

    #[test]
    fn resolve_parameter_ref_not_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        assert!(
            spec.resolve_parameter_ref("#/components/parameters/Missing")
                .is_none()
        );
    }

    #[test]
    fn resolve_request_body_ref_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let body = spec.resolve_request_body_ref("#/components/requestBodies/PetBody");
        assert!(body.is_some());
        assert!(body.unwrap().required);
    }

    #[test]
    fn resolve_response_ref_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        let resp = spec.resolve_response_ref("#/components/responses/NotFound");
        assert!(resp.is_some());
        assert_eq!(
            resp.unwrap().description.as_deref(),
            Some("The requested resource was not found")
        );
    }

    #[test]
    fn resolve_response_ref_not_found() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        assert!(
            spec.resolve_response_ref("#/components/responses/Gone")
                .is_none()
        );
    }

    // -- Schema edge cases --

    #[test]
    fn parse_schema_with_all_of() {
        let yaml = r##"
info:
  title: AllOf Test
  version: "1.0.0"
paths: {}
components:
  schemas:
    Extended:
      allOf:
        - $ref: "#/components/schemas/Base"
        - type: object
          properties:
            extra:
              type: string
    Base:
      type: object
      required:
        - id
      properties:
        id:
          type: integer
"##;
        let spec: OpenApiSpec = serde_yaml_ng::from_str(yaml).unwrap();
        let extended = &spec.components.as_ref().unwrap().schemas["Extended"];
        assert_eq!(extended.all_of.len(), 2);
        assert_eq!(
            extended.all_of[0].ref_path.as_deref(),
            Some("#/components/schemas/Base")
        );
    }

    #[test]
    fn parse_schema_with_nullable() {
        let yaml = r#"
info:
  title: Nullable Test
  version: "1.0.0"
paths: {}
components:
  schemas:
    NullableField:
      type: object
      properties:
        value:
          type: string
          nullable: true
"#;
        let spec: OpenApiSpec = serde_yaml_ng::from_str(yaml).unwrap();
        let schema = &spec.components.as_ref().unwrap().schemas["NullableField"];
        assert!(schema.properties["value"].nullable);
    }

    #[test]
    fn parse_schema_with_additional_properties() {
        let yaml = r#"
info:
  title: AdditionalProps Test
  version: "1.0.0"
paths: {}
components:
  schemas:
    FreeForm:
      type: object
      additionalProperties:
        type: string
"#;
        let spec: OpenApiSpec = serde_yaml_ng::from_str(yaml).unwrap();
        let schema = &spec.components.as_ref().unwrap().schemas["FreeForm"];
        assert!(schema.additional_properties.is_some());
        let inner = schema.additional_properties.as_ref().unwrap();
        assert_eq!(inner.schema_type.as_deref(), Some("string"));
    }

    #[test]
    fn parse_schema_array_with_items() {
        let yaml = r#"
info:
  title: Array Test
  version: "1.0.0"
paths: {}
components:
  schemas:
    StringList:
      type: array
      items:
        type: string
"#;
        let spec: OpenApiSpec = serde_yaml_ng::from_str(yaml).unwrap();
        let schema = &spec.components.as_ref().unwrap().schemas["StringList"];
        assert_eq!(schema.schema_type.as_deref(), Some("array"));
        let items = schema.items.as_ref().unwrap();
        assert_eq!(items.schema_type.as_deref(), Some("string"));
    }

    #[test]
    fn parse_servers() {
        let spec: OpenApiSpec = serde_yaml_ng::from_str(FULL_SPEC_YAML).unwrap();
        assert_eq!(spec.servers.len(), 1);
        assert_eq!(spec.servers[0].url, "https://api.petstore.example.com/v2");
        assert_eq!(
            spec.servers[0].description.as_deref(),
            Some("Production server")
        );
    }

    #[test]
    fn parse_api_key_security_scheme() {
        let yaml = r#"
info:
  title: ApiKey Test
  version: "1.0.0"
paths: {}
components:
  securitySchemes:
    apiKey:
      type: apiKey
      in: header
      name: X-API-Key
"#;
        let spec: OpenApiSpec = serde_yaml_ng::from_str(yaml).unwrap();
        let schemes = &spec.components.as_ref().unwrap().security_schemes;
        let key = &schemes["apiKey"];
        assert_eq!(key.scheme_type, "apiKey");
        assert_eq!(key.location.as_deref(), Some("header"));
        assert_eq!(key.name.as_deref(), Some("X-API-Key"));
    }
}
