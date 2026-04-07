use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod r#gen;
pub mod ir;
pub mod spec;

#[derive(Parser)]
#[command(name = "mcp-forge", version, about = "Generate Rust MCP servers from OpenAPI specs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate a complete Rust MCP server project from an `OpenAPI` spec
    Generate {
        /// Path to the `OpenAPI` 3.0.3 YAML or JSON spec file
        #[arg(long, short)]
        spec: PathBuf,

        /// Output directory for the generated project
        #[arg(long, short, default_value = ".")]
        output: PathBuf,

        /// Project name override (defaults to spec `info.title`, snake-cased)
        #[arg(long)]
        name: Option<String>,
    },

    /// Parse and display the intermediate representation (for debugging)
    Inspect {
        /// Path to the `OpenAPI` spec file
        #[arg(long, short)]
        spec: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Generate { spec, output, name } => {
            let content = std::fs::read_to_string(&spec)
                .with_context(|| format!("failed to read spec: {}", spec.display()))?;

            let openapi: spec::OpenApiSpec = if spec.extension().is_some_and(|e| e == "json") {
                serde_json::from_str(&content)?
            } else {
                serde_yaml_ng::from_str(&content)?
            };

            let mut api = ir::ApiSpec::from_openapi(&openapi);

            if let Some(n) = name {
                api.name = n;
            }

            r#gen::generate(&api, &output).context("failed to generate project")?;

            tracing::info!(
                "Generated MCP server '{}' ({} operations, {} types) → {}",
                api.name,
                api.operations.len(),
                api.types.len(),
                output.display()
            );
        }

        Command::Inspect { spec } => {
            let content = std::fs::read_to_string(&spec)
                .with_context(|| format!("failed to read spec: {}", spec.display()))?;

            let openapi: spec::OpenApiSpec = if spec.extension().is_some_and(|e| e == "json") {
                serde_json::from_str(&content)?
            } else {
                serde_yaml_ng::from_str(&content)?
            };

            let api = ir::ApiSpec::from_openapi(&openapi);

            println!("Name: {}", api.name);
            println!("Version: {}", api.version);
            println!("Base URL: {}", api.base_url.as_deref().unwrap_or("-"));
            println!("Auth: {:?}", api.auth);
            println!("\nOperations ({}):", api.operations.len());
            for op in &api.operations {
                println!(
                    "  {} {} → {} ({})",
                    op.method,
                    op.path,
                    op.id,
                    op.summary.as_deref().unwrap_or("-")
                );
            }
            println!("\nTypes ({}):", api.types.len());
            for t in &api.types {
                if t.is_enum {
                    println!("  enum {} ({} variants)", t.rust_name, t.enum_variants.len());
                } else {
                    println!("  struct {} ({} fields)", t.rust_name, t.fields.len());
                }
            }
        }
    }

    Ok(())
}
