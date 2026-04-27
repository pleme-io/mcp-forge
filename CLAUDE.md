# MCP Forge — Generate Rust MCP Servers from OpenAPI Specs

> **★★★ CSE / Knowable Construction.** This repo operates under **Constructive Substrate Engineering** — canonical specification at [`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md). The Compounding Directive (operational rules: solve once, load-bearing fixes only, idiom-first, models stay current, direction beats velocity) is in the org-level pleme-io/CLAUDE.md ★★★ section. Read both before non-trivial changes.


## Build & Test

```bash
cargo build
cargo run -- inspect --spec path/to/openapi.yaml
cargo run -- generate --spec path/to/openapi.yaml --output /tmp/test --name my-api
```

## Architecture

Code generator that reads an OpenAPI 3.0.3 spec and produces a complete Rust MCP
server project. Three-layer pipeline: spec parser -> intermediate representation -> code generators.

### Modules

| Module | Purpose |
|--------|---------|
| `spec.rs` | OpenAPI 3.0.3 parser (serde types, `$ref` resolution) |
| `ir.rs` | Intermediate representation: `ApiSpec`, `Operation`, `TypeDef`, `AuthMethod` |
| `gen/mod.rs` | Top-level `generate()` function — writes all files to output dir |
| `gen/types.rs` | Generate `src/api/types.rs` — serde structs/enums from IR |
| `gen/client.rs` | Generate `src/client.rs` — typed HTTP client with auth |
| `gen/mcp.rs` | Generate `src/mcp.rs` — MCP tools via `#[tool_router]` (rmcp 0.15) |
| `gen/format.rs` | Generate `src/format.rs` — text formatters per response type |
| `gen/scaffold.rs` | Generate boilerplate: main.rs, error.rs, config.rs, auth.rs, Cargo.toml, flake.nix, module/default.nix, .gitignore |

### Generated Project Structure (13 files)

```
output/
  src/
    main.rs          — Dual-mode: CLI (clap) + MCP server (rmcp stdio)
    api/types.rs     — All request/response types with serde + schemars
    api/mod.rs       — Module declarations
    client.rs        — HTTP client with auth (Bearer/Basic/ApiKey)
    mcp.rs           — MCP tools (1 per operation, #[tool_router])
    format.rs        — Text formatters (1 per response type)
    config.rs        — Config loading with {APP}_CONFIG env
    auth.rs          — API key resolution (flag > env > file)
    error.rs         — thiserror error enum
  Cargo.toml         — All deps (rmcp, reqwest, schemars, clap, serde, tokio)
  flake.nix          — substrate rust-tool-release-flake + crateOverrides
  module/default.nix — HM module with mkMcpOptions
  .gitignore
```

### OpenAPI -> IR Mapping

| OpenAPI Concept | IR Type | Generated Code |
|-----------------|---------|----------------|
| `operationId` | `Operation.id` | MCP tool + CLI command + client method |
| `summary` | `Operation.summary` | `#[tool(description)]` + `/// doc comment` |
| `parameters` (path) | `OpParameter { location: Path }` | URL interpolation in client |
| `parameters` (query) | `OpParameter { location: Query }` | URL query encoding in client |
| `requestBody` | `OpRequestBody.fields` | MCP input struct + CLI args |
| `responses.200.schema` | `Operation.response_type` | Return type + format function |
| `securitySchemes` | `ApiSpec.auth` | Bearer/Basic/ApiKey auth in client |
| `servers[0].url` | `ApiSpec.base_url` | Default base URL in config |
| `components.schemas` | `TypeDef` | Rust struct/enum with serde derives |

### CLI Commands

| Command | Purpose |
|---------|---------|
| `generate` | Generate complete Rust MCP server project |
| `inspect` | Parse spec and display IR summary (for debugging) |

### Integration with forge-gen

Registered in forge-gen as `Category::Mcp` with generator name `mcp-rust`.
forge-gen invokes `mcp-forge generate` as an external tool (same pattern as iac-forge).

```bash
forge-gen generate --spec openapi.yaml --mcp mcp-rust --mcp-name my-api
```

## Design Decisions

- **No template engine** — generators use `format!()` and string building (structure is deterministic)
- **heck for naming** — `ToSnakeCase`, `ToUpperCamelCase` for consistent Rust naming from OpenAPI
- **Forward compatibility** — generated types include `#[serde(flatten)] extra: serde_json::Value`
- **Generated code follows kurage patterns** — Bearer auth, dual-mode CLI+MCP, format module
- **Auth detection** — inferred from `securitySchemes` (Bearer, Basic, API key header)
