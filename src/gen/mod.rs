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
