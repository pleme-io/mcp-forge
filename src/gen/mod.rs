pub mod client;
pub mod format;
pub mod mcp;
pub mod scaffold;
pub mod types;

use crate::ir::ApiSpec;
use anyhow::Result;
use std::path::Path;

/// Generate a complete Rust MCP server project from an API specification.
///
/// Produces the following directory structure under `output_dir`:
/// ```text
/// Cargo.toml
/// flake.nix
/// .gitignore
/// module/default.nix
/// src/
///   main.rs
///   error.rs
///   config.rs
///   auth.rs
///   client.rs
///   mcp.rs
///   format.rs
///   api/
///     mod.rs
///     types.rs
/// ```
///
/// # Errors
///
/// Returns an error if directory creation or file writes fail.
pub fn generate(spec: &ApiSpec, output_dir: &Path) -> Result<()> {
    use std::fs;

    // Create directory structure
    let src_dir = output_dir.join("src");
    let api_dir = src_dir.join("api");
    let module_dir = output_dir.join("module");

    fs::create_dir_all(&api_dir)?;
    fs::create_dir_all(&module_dir)?;

    // Generate scaffold files (Cargo.toml, main.rs, error.rs, config.rs, auth.rs, etc.)
    for (path, content) in scaffold::generate_scaffold(spec) {
        let file_path = output_dir.join(&path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file_path, content)?;
    }

    // Generate src/api/types.rs
    fs::write(api_dir.join("types.rs"), types::generate(spec))?;

    // Generate src/client.rs
    fs::write(src_dir.join("client.rs"), client::generate(spec))?;

    // Generate src/mcp.rs
    fs::write(src_dir.join("mcp.rs"), mcp::generate(spec))?;

    // Generate src/format.rs
    fs::write(src_dir.join("format.rs"), format::generate(spec))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        AuthMethod, EnumVariant, ErrorResponse, FieldDef, HttpMethod, OpParameter,
        OpRequestBody, Operation, ParamLocation, RustType, TypeDef,
    };

    /// Build a realistic `ApiSpec` for end-to-end generation tests.
    fn make_petstore_spec() -> ApiSpec {
        ApiSpec {
            name: "PetStore".into(),
            description: Some("A sample pet store API.".into()),
            version: "1.0.0".into(),
            base_url: Some("https://api.petstore.example.com/v2".into()),
            auth: AuthMethod::Bearer,
            operations: vec![
                Operation {
                    id: "list_pets".into(),
                    method: HttpMethod::Get,
                    path: "/pets".into(),
                    summary: Some("List all pets".into()),
                    description: None,
                    parameters: vec![OpParameter {
                        name: "limit".into(),
                        rust_name: "limit".into(),
                        location: ParamLocation::Query,
                        required: false,
                        rust_type: RustType::Option(Box::new(RustType::I64)),
                        description: Some("Max items to return".into()),
                    }],
                    request_body: None,
                    response_type: Some(RustType::Vec(Box::new(RustType::Named(
                        "Pet".into(),
                    )))),
                    errors: vec![],
                },
                Operation {
                    id: "get_pet".into(),
                    method: HttpMethod::Get,
                    path: "/pets/{petId}".into(),
                    summary: Some("Get a pet by ID".into()),
                    description: None,
                    parameters: vec![OpParameter {
                        name: "petId".into(),
                        rust_name: "pet_id".into(),
                        location: ParamLocation::Path,
                        required: true,
                        rust_type: RustType::String,
                        description: None,
                    }],
                    request_body: None,
                    response_type: Some(RustType::Named("Pet".into())),
                    errors: vec![ErrorResponse {
                        status_code: "404".into(),
                        description: Some("Not found".into()),
                    }],
                },
                Operation {
                    id: "create_pet".into(),
                    method: HttpMethod::Post,
                    path: "/pets".into(),
                    summary: Some("Create a pet".into()),
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
                                description: Some("The pet name".into()),
                                default_value: None,
                            },
                            FieldDef {
                                name: "tag".into(),
                                rust_name: "tag".into(),
                                rust_type: RustType::Option(Box::new(RustType::String)),
                                required: false,
                                description: None,
                                default_value: None,
                            },
                        ],
                        type_name: Some("CreatePetRequest".into()),
                    }),
                    response_type: Some(RustType::Named("Pet".into())),
                    errors: vec![],
                },
                Operation {
                    id: "delete_pet".into(),
                    method: HttpMethod::Delete,
                    path: "/pets/{petId}".into(),
                    summary: Some("Delete a pet".into()),
                    description: None,
                    parameters: vec![OpParameter {
                        name: "petId".into(),
                        rust_name: "pet_id".into(),
                        location: ParamLocation::Path,
                        required: true,
                        rust_type: RustType::String,
                        description: None,
                    }],
                    request_body: None,
                    response_type: None,
                    errors: vec![],
                },
            ],
            types: vec![
                TypeDef {
                    name: "Pet".into(),
                    rust_name: "Pet".into(),
                    fields: vec![
                        FieldDef {
                            name: "id".into(),
                            rust_name: "id".into(),
                            rust_type: RustType::I64,
                            required: true,
                            description: None,
                            default_value: None,
                        },
                        FieldDef {
                            name: "name".into(),
                            rust_name: "name".into(),
                            rust_type: RustType::String,
                            required: true,
                            description: None,
                            default_value: None,
                        },
                        FieldDef {
                            name: "tag".into(),
                            rust_name: "tag".into(),
                            rust_type: RustType::Option(Box::new(RustType::String)),
                            required: false,
                            description: None,
                            default_value: None,
                        },
                        FieldDef {
                            name: "status".into(),
                            rust_name: "status".into(),
                            rust_type: RustType::Option(Box::new(RustType::Named(
                                "PetStatus".into(),
                            ))),
                            required: false,
                            description: None,
                            default_value: None,
                        },
                    ],
                    is_enum: false,
                    enum_variants: vec![],
                    description: Some("A pet in the store.".into()),
                },
                TypeDef {
                    name: "PetStatus".into(),
                    rust_name: "PetStatus".into(),
                    fields: vec![],
                    is_enum: true,
                    enum_variants: vec![
                        EnumVariant {
                            name: "available".into(),
                            rust_name: "Available".into(),
                        },
                        EnumVariant {
                            name: "pending".into(),
                            rust_name: "Pending".into(),
                        },
                        EnumVariant {
                            name: "sold".into(),
                            rust_name: "Sold".into(),
                        },
                    ],
                    description: None,
                },
                TypeDef {
                    name: "CreatePetRequest".into(),
                    rust_name: "CreatePetRequest".into(),
                    fields: vec![
                        FieldDef {
                            name: "name".into(),
                            rust_name: "name".into(),
                            rust_type: RustType::String,
                            required: true,
                            description: Some("The pet name".into()),
                            default_value: None,
                        },
                        FieldDef {
                            name: "tag".into(),
                            rust_name: "tag".into(),
                            rust_type: RustType::Option(Box::new(RustType::String)),
                            required: false,
                            description: None,
                            default_value: None,
                        },
                    ],
                    is_enum: false,
                    enum_variants: vec![],
                    description: None,
                },
            ],
        }
    }

    #[test]
    fn generate_creates_directory_structure() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        // Check directory structure
        assert!(dir.path().join("src").is_dir());
        assert!(dir.path().join("src/api").is_dir());
        assert!(dir.path().join("module").is_dir());
    }

    #[test]
    fn generate_creates_all_expected_files() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let expected_files = [
            "Cargo.toml",
            "flake.nix",
            ".gitignore",
            "module/default.nix",
            "src/main.rs",
            "src/error.rs",
            "src/config.rs",
            "src/auth.rs",
            "src/api/mod.rs",
            "src/api/types.rs",
            "src/client.rs",
            "src/mcp.rs",
            "src/format.rs",
        ];

        for file in &expected_files {
            let path = dir.path().join(file);
            assert!(
                path.exists(),
                "expected file not found: {}",
                path.display()
            );
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(
                !content.is_empty(),
                "file is empty: {}",
                path.display()
            );
        }
    }

    #[test]
    fn generated_cargo_toml_has_correct_name() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(content.contains("name = \"pet_store\""));
        assert!(content.contains("version = \"1.0.0\""));
        assert!(content.contains("edition = \"2024\""));
        assert!(content.contains("rmcp"));
        assert!(content.contains("reqwest"));
        assert!(content.contains("schemars"));
    }

    #[test]
    fn generated_types_rs_has_structs_and_enums() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/api/types.rs")).unwrap();
        assert!(content.contains("pub struct Pet {"));
        assert!(content.contains("pub enum PetStatus {"));
        assert!(content.contains("pub struct CreatePetRequest {"));
        assert!(content.contains("use serde::{Deserialize, Serialize};"));
    }

    #[test]
    fn generated_client_rs_has_methods() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/client.rs")).unwrap();
        assert!(content.contains("pub struct PetStoreClient {"));
        assert!(content.contains("pub async fn list_pets("));
        assert!(content.contains("pub async fn get_pet("));
        assert!(content.contains("pub async fn create_pet("));
        assert!(content.contains("pub async fn delete_pet("));
        assert!(content.contains(".bearer_auth(&self.api_key)"));
    }

    #[test]
    fn generated_mcp_rs_has_tools() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/mcp.rs")).unwrap();
        assert!(content.contains("struct PetStoreMcp {"));
        assert!(content.contains("#[tool_router]"));
        assert!(content.contains("#[tool_handler]"));
        assert!(content.contains("async fn list_pets("));
        assert!(content.contains("async fn get_pet("));
        assert!(content.contains("async fn create_pet("));
        assert!(content.contains("async fn delete_pet("));
    }

    #[test]
    fn generated_format_rs_has_formatters() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/format.rs")).unwrap();
        assert!(content.contains("pub fn truncate("));
        assert!(content.contains("pub fn format_list_pets("));
        assert!(content.contains("pub fn format_get_pet("));
        assert!(content.contains("pub fn format_create_pet("));
        // delete_pet should not have a format function (it's a simple action)
        assert!(!content.contains("format_delete_pet"));
    }

    #[test]
    fn generated_error_rs_uses_spec_name() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/error.rs")).unwrap();
        assert!(content.contains("pub enum PetStoreError {"));
        assert!(content.contains("PET_STORE_API_KEY"));
    }

    #[test]
    fn generated_config_rs_has_base_url() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/config.rs")).unwrap();
        assert!(content.contains("PetStoreConfig"));
        assert!(content.contains("https://api.petstore.example.com/v2"));
    }

    #[test]
    fn generated_auth_rs_references_config() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/auth.rs")).unwrap();
        assert!(content.contains("PetStoreConfig"));
        assert!(content.contains("PetStoreError"));
        assert!(content.contains("PET_STORE_API_KEY"));
        assert!(content.contains("pub fn resolve_api_key"));
    }

    #[test]
    fn generated_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("/target"));
    }

    #[test]
    fn generated_api_mod_rs() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/api/mod.rs")).unwrap();
        assert!(content.contains("pub mod types;"));
    }

    #[test]
    fn generate_with_no_operations() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ApiSpec {
            name: "EmptyApi".into(),
            description: None,
            version: "0.1.0".into(),
            base_url: None,
            auth: AuthMethod::None,
            operations: vec![],
            types: vec![],
        };
        generate(&spec, dir.path()).unwrap();
        // All files should still be created
        assert!(dir.path().join("Cargo.toml").exists());
        assert!(dir.path().join("src/main.rs").exists());
        assert!(dir.path().join("src/client.rs").exists());
        assert!(dir.path().join("src/mcp.rs").exists());
    }

    #[test]
    fn generate_overwrites_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();

        // Generate twice -- should not fail
        generate(&spec, dir.path()).unwrap();
        generate(&spec, dir.path()).unwrap();

        assert!(dir.path().join("Cargo.toml").exists());
    }

    // -- Generated files are valid UTF-8 and non-trivially sized --

    #[test]
    fn generated_files_have_reasonable_size() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let key_files = [
            "src/api/types.rs",
            "src/client.rs",
            "src/mcp.rs",
            "src/format.rs",
        ];

        for file in &key_files {
            let content = std::fs::read_to_string(dir.path().join(file)).unwrap();
            assert!(
                content.len() > 100,
                "{file} should have substantial content, got {} bytes",
                content.len()
            );
        }
    }

    // -- Generated mcp.rs and client.rs reference the same operation names --

    #[test]
    fn generated_mcp_and_client_share_operation_names() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let mcp = std::fs::read_to_string(dir.path().join("src/mcp.rs")).unwrap();
        let client = std::fs::read_to_string(dir.path().join("src/client.rs")).unwrap();

        for op in &spec.operations {
            assert!(
                client.contains(&format!("fn {}(", op.id)),
                "client.rs missing operation: {}",
                op.id
            );
            assert!(
                mcp.contains(&format!("fn {}(", op.id)),
                "mcp.rs missing operation: {}",
                op.id
            );
        }
    }

    // -- No operations still generates compilable scaffold --

    #[test]
    fn empty_spec_generates_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ApiSpec {
            name: "EmptyApi".into(),
            description: None,
            version: "0.1.0".into(),
            base_url: None,
            auth: AuthMethod::None,
            operations: vec![],
            types: vec![],
        };
        generate(&spec, dir.path()).unwrap();

        let expected = [
            "Cargo.toml",
            "src/main.rs",
            "src/error.rs",
            "src/config.rs",
            "src/auth.rs",
            "src/api/mod.rs",
            "src/api/types.rs",
            "src/client.rs",
            "src/mcp.rs",
            "src/format.rs",
            "flake.nix",
            "module/default.nix",
            ".gitignore",
        ];

        for file in &expected {
            assert!(
                dir.path().join(file).exists(),
                "empty spec should still generate: {file}"
            );
        }
    }

    // -- Types rs has correct enum from spec --

    #[test]
    fn generated_types_rs_has_enum_variants() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/api/types.rs")).unwrap();
        assert!(content.contains("Available"));
        assert!(content.contains("Pending"));
        assert!(content.contains("Sold"));
    }

    // -- format.rs skips delete operations --

    #[test]
    fn generated_format_rs_skips_delete() {
        let dir = tempfile::tempdir().unwrap();
        let spec = make_petstore_spec();
        generate(&spec, dir.path()).unwrap();

        let content = std::fs::read_to_string(dir.path().join("src/format.rs")).unwrap();
        assert!(
            !content.contains("format_delete"),
            "format.rs should not contain delete formatters"
        );
    }

    // -- Different auth methods produce different client code --

    #[test]
    fn generate_with_basic_auth() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = make_petstore_spec();
        spec.auth = AuthMethod::Basic;
        generate(&spec, dir.path()).unwrap();

        let client = std::fs::read_to_string(dir.path().join("src/client.rs")).unwrap();
        assert!(client.contains("basic_auth"));
    }

    #[test]
    fn generate_with_api_key_auth() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = make_petstore_spec();
        spec.auth = AuthMethod::ApiKeyHeader("X-Custom-Key".into());
        generate(&spec, dir.path()).unwrap();

        let client = std::fs::read_to_string(dir.path().join("src/client.rs")).unwrap();
        assert!(client.contains("X-Custom-Key"));
    }

    #[test]
    fn generate_with_no_auth() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = make_petstore_spec();
        spec.auth = AuthMethod::None;
        generate(&spec, dir.path()).unwrap();

        let client = std::fs::read_to_string(dir.path().join("src/client.rs")).unwrap();
        assert!(!client.contains("bearer_auth"));
        assert!(!client.contains("basic_auth"));
    }
}
