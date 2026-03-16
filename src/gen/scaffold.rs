use crate::ir::ApiSpec;
use heck::{ToSnakeCase, ToUpperCamelCase};

/// Generate all scaffold/boilerplate files for the MCP server project.
///
/// Returns a list of `(relative_path, content)` pairs. The caller writes
/// each pair to `output_dir / relative_path`.
///
/// Generated files:
/// - `Cargo.toml` -- dependencies (rmcp, reqwest, schemars, clap, serde, tokio, etc.)
/// - `src/main.rs` -- dual-mode entry point (CLI + MCP server)
/// - `src/error.rs` -- thiserror enum
/// - `src/config.rs` -- shikumi-style config loading with `{APP}_CONFIG` env
/// - `src/auth.rs` -- API key resolution (flag > env > file)
/// - `src/api/mod.rs` -- module declaration
/// - `flake.nix` -- substrate pattern with `crateOverrides` for rmcp
/// - `module/default.nix` -- home-manager module with `mkMcpOptions`
/// - `.gitignore`
pub fn generate_scaffold(spec: &ApiSpec) -> Vec<(String, String)> {
    let mut files = Vec::with_capacity(10);

    files.push(("Cargo.toml".into(), generate_cargo_toml(spec)));
    files.push(("src/main.rs".into(), generate_main_rs(spec)));
    files.push(("src/error.rs".into(), generate_error_rs(spec)));
    files.push(("src/config.rs".into(), generate_config_rs(spec)));
    files.push(("src/auth.rs".into(), generate_auth_rs(spec)));
    files.push(("src/api/mod.rs".into(), generate_api_mod_rs()));
    files.push(("flake.nix".into(), generate_flake_nix(spec)));
    files.push((
        "module/default.nix".into(),
        generate_module_nix(spec),
    ));
    files.push((".gitignore".into(), generate_gitignore()));

    files
}

fn generate_cargo_toml(spec: &ApiSpec) -> String {
    let name = spec.name.to_snake_case();
    let default_description = format!("Rust CLI + MCP server for {}", spec.name);
    let description = spec
        .description
        .as_deref()
        .unwrap_or(&default_description);
    let version = &spec.version;

    format!(
        r#"[package]
name = "{name}"
version = "{version}"
edition = "2024"
rust-version = "1.89.0"
description = "{description}"
license = "MIT"

[[bin]]
name = "{name}"
path = "src/main.rs"

[dependencies]
anyhow = "1"
clap = {{ version = "4", features = ["derive"] }}
heck = "0.5"
reqwest = {{ version = "0.12", features = ["json", "rustls-tls"], default-features = false }}
rmcp = {{ version = "0.15", features = ["server", "transport-io"] }}
schemars = "0.8"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
serde_yaml_ng = "0.10"
thiserror = "2"
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}
tracing = "0.1"
tracing-subscriber = {{ version = "0.3", features = ["env-filter", "json"] }}
urlencoding = "2"

[profile.release]
codegen-units = 1
lto = true
opt-level = "z"
strip = true

[lints.clippy]
pedantic = "warn"
"#
    )
}

fn generate_main_rs(spec: &ApiSpec) -> String {
    let name = spec.name.to_snake_case();
    let config_type = format!("{}Config", spec.name.to_upper_camel_case());
    let default_description = format!("{name} CLI + MCP server");
    let description = spec
        .description
        .as_deref()
        .unwrap_or(&default_description);

    format!(
        r#"use clap::Parser;
use std::process::ExitCode;

mod api;
mod auth;
mod client;
mod config;
mod error;
mod format;
mod mcp;

use config::{config_type};

#[derive(Parser)]
#[command(name = "{name}", about = "{description}")]
struct Cli {{
    /// Run in MCP server mode (default when no subcommand given)
    #[command(subcommand)]
    command: Option<Command>,

    /// API key (overrides env and config file)
    #[arg(long)]
    api_key: Option<String>,

    /// API base URL (overrides config)
    #[arg(long)]
    api_url: Option<String>,
}}

#[derive(clap::Subcommand)]
enum Command {{
    /// Run the MCP server on stdio
    Serve,
}}

#[tokio::main]
async fn main() -> ExitCode {{
    let cli = Cli::parse();

    // No subcommand or explicit serve -> MCP server mode (stdio)
    match cli.command {{
        None | Some(Command::Serve) => {{
            init_tracing(true);
            if let Err(e) = mcp::run().await {{
                eprintln!("MCP server error: {{e}}");
                return ExitCode::FAILURE;
            }}
            ExitCode::SUCCESS
        }}
    }}
}}

fn init_tracing(json: bool) {{
    use tracing_subscriber::{{EnvFilter, fmt}};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    if json {{
        fmt().json().with_env_filter(filter).with_writer(std::io::stderr).init();
    }} else {{
        fmt().with_env_filter(filter).init();
    }}
}}
"#,
    )
}

fn generate_error_rs(spec: &ApiSpec) -> String {
    let error_name = format!("{}Error", spec.name.to_upper_camel_case());
    let env_var = format!(
        "{}_API_KEY",
        spec.name.to_snake_case().to_uppercase()
    );

    format!(
        r#"use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum {error_name} {{
    #[error("HTTP request failed: {{0}}")]
    Request(#[from] reqwest::Error),

    #[error("API returned {{status}}: {{body}}")]
    Api {{ status: u16, body: String }},

    #[error("JSON parse error: {{0}}")]
    Json(#[from] serde_json::Error),

    #[error("API key not found -- set --api-key, {env_var}, or create {{path}}")]
    NoApiKey {{ path: PathBuf }},
}}

pub type Result<T> = std::result::Result<T, {error_name}>;
"#
    )
}

fn generate_config_rs(spec: &ApiSpec) -> String {
    let config_type = format!("{}Config", spec.name.to_upper_camel_case());
    let app_name = spec.name.to_snake_case();
    let app_upper = app_name.to_uppercase();
    let base_url = spec
        .base_url
        .as_deref()
        .unwrap_or("https://api.example.com");

    format!(
        r#"use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct {config_type} {{
    pub api_url: String,
    pub api_key_file: PathBuf,
}}

impl Default for {config_type} {{
    fn default() -> Self {{
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        Self {{
            api_url: "{base_url}".into(),
            api_key_file: PathBuf::from(&home).join(".config/{app_name}/api-key"),
        }}
    }}
}}

impl {config_type} {{
    pub fn load() -> Self {{
        // Priority:
        // 1. {app_upper}_CONFIG env (set by Nix HM module for MCP server context)
        // 2. XDG_CONFIG_HOME/{app_name}/{app_name}.yaml
        // 3. ~/.config/{app_name}/{app_name}.yaml
        // 4. Defaults

        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());

        let candidates: Vec<PathBuf> = [
            // Nix module sets this for MCP server processes that lack user env
            std::env::var("{app_upper}_CONFIG").map(PathBuf::from).ok(),
            std::env::var("XDG_CONFIG_HOME")
                .map(|x| PathBuf::from(x).join("{app_name}/{app_name}.yaml"))
                .ok(),
            Some(PathBuf::from(&home).join(".config/{app_name}/{app_name}.yaml")),
        ]
        .into_iter()
        .flatten()
        .collect();

        for candidate in &candidates {{
            if candidate.exists() {{
                if let Ok(content) = std::fs::read_to_string(candidate) {{
                    match serde_yaml_ng::from_str::<Self>(&content) {{
                        Ok(config) => return config,
                        Err(e) => {{
                            tracing::warn!("failed to parse {{}}: {{e}}", candidate.display());
                        }}
                    }}
                }}
            }}
        }}

        Self::default()
    }}
}}
"#
    )
}

fn generate_auth_rs(spec: &ApiSpec) -> String {
    let config_type = format!("{}Config", spec.name.to_upper_camel_case());
    let error_name = format!("{}Error", spec.name.to_upper_camel_case());
    let env_var = format!(
        "{}_API_KEY",
        spec.name.to_snake_case().to_uppercase()
    );

    format!(
        r#"use crate::config::{config_type};
use crate::error::{{{error_name}, Result}};
use std::path::PathBuf;

/// Resolve the API key from (in priority order):
/// 1. Explicit CLI flag value
/// 2. {env_var} environment variable
/// 3. Contents of the configured api_key_file
pub fn resolve_api_key(explicit: Option<&str>, config: &{config_type}) -> Result<String> {{
    // 1. Explicit flag
    if let Some(key) = explicit {{
        return Ok(key.to_string());
    }}

    // 2. Environment variable
    if let Ok(key) = std::env::var("{env_var}") {{
        if !key.is_empty() {{
            return Ok(key);
        }}
    }}

    // 3. File
    let path = expand_tilde(&config.api_key_file);
    match std::fs::read_to_string(&path) {{
        Ok(content) => {{
            let key = content.trim().to_string();
            if key.is_empty() {{
                Err({error_name}::NoApiKey {{ path }})
            }} else {{
                Ok(key)
            }}
        }}
        Err(_) => Err({error_name}::NoApiKey {{ path }}),
    }}
}}

fn expand_tilde(path: &PathBuf) -> PathBuf {{
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {{
        if let Ok(home) = std::env::var("HOME") {{
            return PathBuf::from(home).join(rest);
        }}
    }}
    path.clone()
}}
"#
    )
}

fn generate_api_mod_rs() -> String {
    "pub mod types;\n".into()
}

fn generate_flake_nix(spec: &ApiSpec) -> String {
    let app_name = spec.name.to_snake_case();
    let default_description = format!("{app_name} -- Rust CLI + MCP server");
    let description = spec
        .description
        .as_deref()
        .unwrap_or(&default_description);

    format!(
        r#"{{
  description = "{app_name} -- {description}";

  nixConfig = {{
    allow-import-from-derivation = true;
  }};

  inputs = {{
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    crate2nix.url = "github:nix-community/crate2nix";
    flake-utils.url = "github:numtide/flake-utils";
    substrate = {{
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    }};
    devenv = {{
      url = "github:cachix/devenv";
      inputs.nixpkgs.follows = "nixpkgs";
    }};
  }};

  outputs = {{
    self,
    nixpkgs,
    crate2nix,
    flake-utils,
    substrate,
    devenv,
  }}:
    (import "${{substrate}}/lib/rust-tool-release-flake.nix" {{
      inherit nixpkgs crate2nix flake-utils devenv;
    }}) {{
      toolName = "{app_name}";
      src = self;
      repo = "pleme-io/{app_name}";
      crateOverrides = {{
        rmcp = attrs: {{
          CARGO_CRATE_NAME = "rmcp";
        }};
      }};
    }}
    // {{
      homeManagerModules.default = import ./module {{
        hmHelpers = import "${{substrate}}/lib/hm-service-helpers.nix" {{ lib = nixpkgs.lib; }};
      }};
    }};
}}
"#
    )
}

fn generate_module_nix(spec: &ApiSpec) -> String {
    let app_name = spec.name.to_snake_case();
    let app_upper = app_name.to_uppercase();
    let config_type_name = &spec.name;
    let base_url = spec
        .base_url
        .as_deref()
        .unwrap_or("https://api.example.com");

    format!(
        r#"# {config_type_name} home-manager module -- MCP server + CLI
#
# Namespace: services.{app_name}.*
#
# Provides:
#   - MCP server entry (consumed by claude/anvil for all AI agents)
#   - CLI binary in PATH
#   - Config file generation (~/.config/{app_name}/{app_name}.yaml)
#   - Env propagation: {app_upper}_CONFIG passed to MCP server process
#
# Usage:
#   services.{app_name}.package = inputs.{app_name}.packages.${{system}}.default;
#   services.{app_name}.enable = true;
#   services.{app_name}.mcp.enable = true;
{{ hmHelpers }}:
{{
  lib,
  config,
  pkgs,
  ...
}}:
with lib; let
  inherit (hmHelpers) mkMcpOptions mkMcpServerEntry;
  cfg = config.services.{app_name};
  mcpCfg = cfg.mcp;
  homeDir = config.home.homeDirectory;

  defaultApiKeyFile = "${{homeDir}}/.config/{app_name}/api-key";

  resolvedApiKeyFile =
    if cfg.settings.apiKeyFile != null
    then cfg.settings.apiKeyFile
    else defaultApiKeyFile;

  configFile = pkgs.writeText "{app_name}.yaml"
    (builtins.toJSON ({{
      api_url = cfg.settings.apiUrl;
      api_key_file = resolvedApiKeyFile;
    }}));

  mcpEnv = optionalAttrs cfg.settings.propagateApiKey {{
    {app_upper}_CONFIG = "${{configFile}}";
  }};
in {{
  options.services.{app_name} = {{
    enable = mkEnableOption "{app_name} -- CLI + MCP server";

    package = mkOption {{
      type = types.package;
      description = ''
        The {app_name} binary package. Must be set explicitly from your flake input:
          services.{app_name}.package = inputs.{app_name}.packages.''${{system}}.default;
      '';
    }};

    mcp = mkMcpOptions {{
      defaultPackage = pkgs.hello;
    }};

    settings = {{
      apiUrl = mkOption {{
        type = types.str;
        default = "{base_url}";
        description = "API base URL.";
      }};

      apiKeyFile = mkOption {{
        type = types.nullOr types.str;
        default = null;
        description = ''
          Path to file containing the API key.
          When null, defaults to ~/.config/{app_name}/api-key.
        '';
      }};

      propagateApiKey = mkOption {{
        type = types.bool;
        default = true;
        description = ''
          Pass config file path to the MCP server process via {app_upper}_CONFIG env.
          Ensures the MCP server can find the API key when launched by Claude
          Code or other MCP clients that don't inherit user environment.
        '';
      }};
    }};
  }};

  config = mkMerge [
    {{
      services.{app_name}.mcp.package = mkDefault cfg.package;
    }}

    (mkIf cfg.enable {{
      home.packages = [ cfg.package ];

      xdg.configFile."{app_name}/{app_name}.yaml".source = configFile;
    }})

    (mkIf mcpCfg.enable {{
      services.{app_name}.mcp.serverEntry = mkMcpServerEntry ({{
        command = "${{mcpCfg.package}}/bin/{app_name}";
      }} // optionalAttrs (mcpEnv != {{}}) {{
        env = mcpEnv;
      }});
    }})
  ];
}}
"#
    )
}

fn generate_gitignore() -> String {
    "/target\n/result\n".into()
}
