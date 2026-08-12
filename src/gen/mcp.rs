//! Emit `src/mcp.rs` — the MCP server — from the API spec.
//!
//! Every construct is built as a [`rust_ast`] tree and rendered once. Per
//! ★★ TYPED EMISSION there is no `format!()` of Rust syntax here; the
//! remaining `format!` calls build *names* and prose, not syntax.

use super::rust_ast::{
    render_file, Attr, Block, Braces, Doc, Expr, FieldDecl, FieldInit, FnDef, FormatTemplate,
    Ident, ImplBlock, ImplItem, Item, MatchArm, Param, Params, Path, Stmt, StrLit, StructDef,
    TypeExpr, UseTree,
};
use crate::ir::{ApiSpec, HttpMethod, OpParameter, Operation, ParamLocation, RustType};
use heck::{ToSnakeCase, ToUpperCamelCase};

// ── Identifier helpers ─────────────────────────────────────────────────────

/// # Panics
///
/// Panics if `name` is not a legal Rust identifier. That indicates an IR bug;
/// there is no sane partial output, so generation stops.
fn id(name: &str) -> Ident {
    Ident::new(name)
        .unwrap_or_else(|e| panic!("code generator built an illegal Rust identifier: {e}"))
}

/// # Panics
///
/// Panics if any segment is not a legal Rust identifier.
fn path(segments: &[&str]) -> Path {
    Path::new(segments)
        .unwrap_or_else(|e| panic!("code generator built an illegal Rust path: {e}"))
}

fn var(name: &str) -> Expr {
    Expr::Path(path(&[name]))
}

fn call(receiver: Expr, method: &str, args: Vec<Expr>) -> Expr {
    Expr::MethodCall(Box::new(receiver), id(method), args)
}

/// `input.<name>`
fn input_field(name: &str) -> Expr {
    Expr::Field(Box::new(var("input")), id(name))
}

/// `|e| e.to_string()`
fn stringify_err() -> Expr {
    Expr::Closure(id("e"), Box::new(call(var("e"), "to_string", vec![])))
}

// ── Entry point ────────────────────────────────────────────────────────────

/// Generate the `src/mcp.rs` file from the API spec.
///
/// Produces:
/// - MCP input structs with `schemars::JsonSchema` for each operation
/// - An MCP server struct with `#[tool_router]` / `#[tool_handler]`
/// - Tool methods that delegate to the client and format results
#[must_use]
pub fn generate(spec: &ApiSpec) -> String {
    let pascal = spec.name.to_upper_camel_case();
    let mcp_struct = format!("{pascal}Mcp");
    let client_type = format!("{pascal}Client");
    let config_type = format!("{pascal}Config");

    let mut items = vec![
        rmcp_import(),
        Item::Use { path: path(&["serde", "Deserialize"]), leaves: vec![], glob: false },
        Item::Blank,
        Item::Use { path: path(&["crate", "auth"]), leaves: vec![], glob: false },
        Item::Use { path: path(&["crate", "client", &client_type]), leaves: vec![], glob: false },
        Item::Use { path: path(&["crate", "config", &config_type]), leaves: vec![], glob: false },
        Item::Use { path: path(&["crate", "format"]), leaves: vec![], glob: false },
        Item::Blank,
        Item::Comment("-- MCP tool input types --".into()),
        Item::Comment(String::new()),
        Item::Comment("Each struct maps to an operation from the API spec. Field descriptions".into()),
        Item::Comment("are preserved for schemars -> MCP tool schema generation.".into()),
        Item::Blank,
    ];

    for op in &spec.operations {
        if let Some(s) = input_struct(op) {
            items.push(Item::Struct(s));
            items.push(Item::Blank);
        }
    }

    items.push(Item::Comment("-- MCP Server --".into()));
    items.push(Item::Blank);

    items.push(Item::Struct(StructDef {
        doc: Doc::default(),
        attrs: vec![Attr::List(id("derive"), vec![path(&["Debug"]), path(&["Clone"])])],
        public: false,
        name: id(&mcp_struct),
        fields: vec![
            FieldDecl {
                attrs: vec![],
                public: false,
                name: id("client"),
                ty: TypeExpr::Path(path(&[&client_type])),
            },
            FieldDecl {
                attrs: vec![],
                public: false,
                name: id("tool_router"),
                ty: TypeExpr::App(
                    path(&["ToolRouter"]),
                    vec![TypeExpr::Path(path(&["Self"]))],
                ),
            },
        ],
    }));
    items.push(Item::Blank);

    let mut members = vec![
        ImplItem::Fn(constructor(&client_type, &config_type)),
        ImplItem::Blank,
    ];
    for op in &spec.operations {
        members.push(ImplItem::Fn(tool_method(op)));
        members.push(ImplItem::Blank);
    }
    items.push(Item::Impl(ImplBlock {
        attrs: vec![Attr::Word(id("tool_router"))],
        trait_path: None,
        self_ty: path(&[&mcp_struct]),
        items: members,
    }));
    items.push(Item::Blank);

    items.push(Item::Impl(ImplBlock {
        attrs: vec![Attr::Word(id("tool_handler"))],
        trait_path: Some(path(&["ServerHandler"])),
        self_ty: path(&[&mcp_struct]),
        items: vec![ImplItem::Fn(get_info(spec))],
    }));
    items.push(Item::Blank);

    items.push(Item::Comment("-- Entry point --".into()));
    items.push(Item::Blank);
    items.push(Item::Fn(run_fn(&mcp_struct)));

    render_file(&items)
}

/// The fixed `rmcp` import block.
fn rmcp_import() -> Item {
    let leaf = |segs: &[&str]| UseTree::Leaf(path(segs));
    Item::UseLines {
        path: path(&["rmcp"]),
        lines: vec![
            vec![leaf(&["ServerHandler"]), leaf(&["ServiceExt"])],
            vec![UseTree::Group(
                path(&["handler", "server"]),
                vec![leaf(&["router", "tool", "ToolRouter"]), leaf(&["wrapper", "Parameters"])],
            )],
            vec![UseTree::Group(
                path(&["model"]),
                vec![leaf(&["ServerCapabilities"]), leaf(&["ServerInfo"])],
            )],
            vec![
                leaf(&["schemars"]),
                leaf(&["tool"]),
                leaf(&["tool_handler"]),
                leaf(&["tool_router"]),
            ],
            vec![leaf(&["transport", "stdio"])],
        ],
    }
}

// ── Input structs ──────────────────────────────────────────────────────────

/// The parameters an operation exposes as MCP tool input.
fn input_params(op: &Operation) -> Vec<&OpParameter> {
    op.parameters
        .iter()
        .filter(|p| p.location == ParamLocation::Path || p.location == ParamLocation::Query)
        .collect()
}

/// Build the input struct for an operation, or `None` when it has no inputs
/// (those tools take `serde_json::Value` instead).
fn input_struct(op: &Operation) -> Option<StructDef> {
    let params = input_params(op);
    let body_fields = op.request_body.as_ref().map(|b| &b.fields);
    let has_body_fields = body_fields.is_some_and(|f| !f.is_empty());

    if params.is_empty() && !has_body_fields {
        return None;
    }

    let described = |description: &Option<String>| {
        description.as_ref().map_or_else(Vec::new, |d| {
            vec![Attr::KeyValue {
                name: path(&["schemars"]),
                key: id("description"),
                value: StrLit::one_line(d.clone()),
            }]
        })
    };

    let mut fields = Vec::new();
    for p in &params {
        fields.push(FieldDecl {
            attrs: described(&p.description),
            public: false,
            name: id(&p.rust_name),
            ty: input_field_ty(&p.rust_type, p.required),
        });
    }
    if let Some(body) = body_fields {
        for f in body {
            fields.push(FieldDecl {
                attrs: described(&f.description),
                public: false,
                name: id(&f.rust_name),
                ty: input_field_ty(&f.rust_type, f.required),
            });
        }
    }

    Some(StructDef {
        doc: Doc::default(),
        attrs: vec![Attr::List(
            id("derive"),
            vec![path(&["Debug"]), path(&["Deserialize"]), path(&["schemars", "JsonSchema"])],
        )],
        public: false,
        name: id(&input_struct_name(op)),
        fields,
    })
}

fn input_struct_name(op: &Operation) -> String {
    format!("{}Input", op.id.to_upper_camel_case())
}

fn input_field_ty(rt: &RustType, required: bool) -> TypeExpr {
    if required || rt.is_option() {
        TypeExpr::Ir(rt.clone())
    } else {
        TypeExpr::App(path(&["Option"]), vec![TypeExpr::Ir(rt.clone())])
    }
}

// ── Server construction ────────────────────────────────────────────────────

fn constructor(client_type: &str, config_type: &str) -> FnDef {
    let load_config = Stmt::Let {
        name: id("config"),
        mutable: false,
        ty: None,
        init: Expr::Call(path(&[config_type, "load"]), vec![]),
    };

    let resolve_key = Stmt::Let {
        name: id("api_key"),
        mutable: false,
        ty: None,
        init: Expr::Try(Box::new(call(
            Expr::Call(
                path(&["auth", "resolve_api_key"]),
                vec![Expr::Path(path(&["None"])), Expr::Ref(Box::new(var("config")))],
            ),
            "map_err",
            vec![stringify_err()],
        ))),
    };

    let build_client = Stmt::LetWrapped {
        name: id("client"),
        init: Expr::Try(Box::new(call(
            Expr::Call(
                path(&[client_type, "new"]),
                vec![
                    Expr::Ref(Box::new(Expr::Field(Box::new(var("config")), id("api_url")))),
                    Expr::Ref(Box::new(var("api_key"))),
                ],
            ),
            "map_err",
            vec![stringify_err()],
        ))),
    };

    let ok_self = Expr::Call(
        path(&["Ok"]),
        vec![Expr::StructLit {
            path: path(&["Self"]),
            fields: vec![
                FieldInit::Shorthand(id("client")),
                FieldInit::Named(
                    id("tool_router"),
                    Expr::Call(path(&["Self", "tool_router"]), vec![]),
                ),
            ],
            braces: Braces::Multiline,
        }],
    );

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: false,
        is_async: false,
        name: id("new"),
        generics: vec![],
        params: vec![],
        params_layout: Params::Inline,
        ret: TypeExpr::App(
            path(&["Result"]),
            vec![TypeExpr::Path(path(&["Self"])), TypeExpr::Ir(RustType::String)],
        ),
        body: Block(vec![
            load_config,
            resolve_key,
            build_client,
            Stmt::Blank,
            Stmt::Tail(ok_self),
        ]),
    }
}

fn get_info(spec: &ApiSpec) -> FnDef {
    let default_instructions = format!("{} MCP server", spec.name);
    let instructions = spec.description.as_deref().unwrap_or(&default_instructions);

    let server_info = Expr::StructLit {
        path: path(&["ServerInfo"]),
        fields: vec![
            FieldInit::Named(
                id("instructions"),
                Expr::CallWrapped {
                    func: path(&["Some"]),
                    arg: Box::new(Expr::Chain {
                        receiver: Box::new(Expr::Str(StrLit::one_line(instructions))),
                        links: vec![super::rust_ast::ChainLink::Call(id("into"), vec![])],
                    }),
                },
            ),
            FieldInit::Named(
                id("capabilities"),
                call(
                    call(
                        Expr::Call(path(&["ServerCapabilities", "builder"]), vec![]),
                        "enable_tools",
                        vec![],
                    ),
                    "build",
                    vec![],
                ),
            ),
            FieldInit::Rest(Expr::Call(path(&["Default", "default"]), vec![])),
        ],
        braces: Braces::Multiline,
    };

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: false,
        is_async: false,
        name: id("get_info"),
        generics: vec![],
        params: vec![Param::SelfRef],
        params_layout: Params::Inline,
        ret: TypeExpr::Path(path(&["ServerInfo"])),
        body: Block(vec![Stmt::Tail(server_info)]),
    }
}

fn run_fn(mcp_struct: &str) -> FnDef {
    // let server = <Mcp>::new()?.serve(stdio()).await?;
    let serve = Stmt::Let {
        name: id("server"),
        mutable: false,
        ty: None,
        init: Expr::Try(Box::new(Expr::Await(Box::new(call(
            Expr::Try(Box::new(Expr::Call(path(&[mcp_struct, "new"]), vec![]))),
            "serve",
            vec![Expr::Call(path(&["stdio"]), vec![])],
        ))))),
    };

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: true,
        is_async: true,
        name: id("run"),
        generics: vec![],
        params: vec![],
        params_layout: Params::Inline,
        ret: TypeExpr::App(
            path(&["std", "result", "Result"]),
            vec![
                TypeExpr::Unit,
                TypeExpr::App(
                    path(&["Box"]),
                    vec![TypeExpr::Dyn(path(&["std", "error", "Error"]))],
                ),
            ],
        ),
        body: Block(vec![
            serve,
            Stmt::Semi(Expr::Try(Box::new(Expr::Await(Box::new(call(
                var("server"),
                "waiting",
                vec![],
            )))))),
            Stmt::Tail(Expr::Call(path(&["Ok"]), vec![Expr::Unit])),
        ]),
    }
}

// ── Tool methods ───────────────────────────────────────────────────────────

/// Split an OpenAPI path into a format template.
///
/// Path parameters become captured holes keyed on their **raw** OpenAPI name.
/// That reproduces the established output exactly — and makes visible a
/// pre-existing defect it contains: the success message for an operation with
/// a path parameter emits `format!("Success: DELETE /x/{xId}")`, a hole naming
/// a variable that is not in scope in the generated file. Modelling it as a
/// real hole records the bug rather than hiding it behind escaped braces.
///
/// A parameter whose name is not a legal Rust identifier could never have
/// compiled; it is emitted as literal text (braces doubled) instead.
fn message_template(prefix: &str, method: HttpMethod, op_path: &str) -> FormatTemplate {
    let mut template = FormatTemplate::new().lit(prefix).lit(method.to_string()).lit(" ");
    let mut rest = op_path;

    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open..].find('}') else {
            break;
        };
        let close = open + close_rel;
        if !rest[..open].is_empty() {
            template = template.lit(&rest[..open]);
        }
        template = match Ident::new(&rest[open + 1..close]) {
            Ok(name) => template.captured(name),
            Err(_) => template.lit(&rest[open..=close]),
        };
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        template = template.lit(rest);
    }
    template
}

/// Arguments passed from the deserialised input struct to the client method.
fn client_args(op: &Operation, has_body: bool) -> Vec<Expr> {
    let mut args = Vec::new();

    for p in op.parameters.iter().filter(|p| p.location == ParamLocation::Path) {
        args.push(Expr::Ref(Box::new(input_field(&p.rust_name))));
    }
    for p in op.parameters.iter().filter(|p| p.location == ParamLocation::Query) {
        if p.rust_type.is_option() {
            args.push(call(input_field(&p.rust_name), "as_deref", vec![]));
        } else {
            args.push(input_field(&p.rust_name));
        }
    }
    if has_body {
        args.push(Expr::Ref(Box::new(var("req"))));
    }
    args
}

fn tool_method(op: &Operation) -> FnDef {
    let method_name = op.id.to_snake_case();

    let default_description = format!("{} operation", op.id);
    let description = op
        .summary
        .as_deref()
        .or(op.description.as_deref())
        .unwrap_or(&default_description);

    let has_params = !op.parameters.is_empty()
        || op.request_body.as_ref().is_some_and(|b| !b.fields.is_empty());
    let has_body = op.request_body.as_ref().is_some_and(|b| !b.fields.is_empty());

    // Parameters(input): Parameters<XInput>  /  Parameters(_): Parameters<serde_json::Value>
    let (binding, input_ty) = if has_params {
        (var("input"), TypeExpr::Path(path(&[&input_struct_name(op)])))
    } else {
        (var("_"), TypeExpr::Ir(RustType::Value))
    };
    let parameters_param = Param::Destructured {
        pattern: Expr::Call(path(&["Parameters"]), vec![binding]),
        ty: TypeExpr::App(path(&["Parameters"]), vec![input_ty]),
    };

    let mut body = Vec::new();
    if has_body {
        body.push(request_body_stmt(op));
    }

    // A delete, or a stop/delete-prefixed operation, has no rich response to
    // format; it reports success instead.
    let snake = op.id.to_snake_case();
    let is_simple_action = matches!(op.method, HttpMethod::Delete)
        || snake.starts_with("stop")
        || snake.starts_with("delete");

    let ok_arm = if is_simple_action {
        MatchArm {
            pattern: Expr::Call(path(&["Ok"]), vec![var("_")]),
            body: Expr::Format(
                message_template("Success: ", op.method, &op.path),
                vec![],
            ),
        }
    } else {
        MatchArm {
            pattern: Expr::Call(path(&["Ok"]), vec![var("result")]),
            body: Expr::Call(
                path(&["format", &format!("format_{method_name}")]),
                vec![Expr::Ref(Box::new(var("result")))],
            ),
        }
    };

    // A failed call is BLIND — the tool could not look — not a sentence.
    //
    // This emitted `format!("Error: {e}")`, so every generated tool answered a
    // failure with English prose. The reader of an MCP answer is a model with
    // no peripheral vision: it has the bytes and nothing else, so "Error:
    // connection refused" and an empty result set are two prose strings it must
    // tell apart by reading. kotae gives the answer a discriminant instead, and
    // `blind` is the honest arm — an HTTP call that did not complete says
    // nothing about whether the resource exists.
    //
    // Deliberately NOT classified finer here. The generator cannot see the
    // status code without inspecting the error type, and a 404 mapped to
    // `empty` by guesswork would be exactly the collapse kotae exists to
    // prevent. Per-operation classification is the consumer's job at the point
    // it knows its own error shape. `pending-mcp-forge: classify-status-codes`.
    let err_arm = MatchArm {
        pattern: Expr::Call(path(&["Err"]), vec![var("e")]),
        body: call(
            Expr::Call(
                path(&["kotae", "Answer", "blind"]),
                vec![call(var("e"), "to_string", vec![])],
            ),
            "render",
            vec![],
        ),
    };

    body.push(Stmt::Match {
        scrutinee: Expr::Await(Box::new(call(
            Expr::Field(Box::new(var("self")), id("client")),
            &method_name,
            client_args(op, has_body),
        ))),
        arms: vec![ok_arm, err_arm],
    });

    FnDef {
        doc: Doc::default(),
        attrs: vec![Attr::KeyValue {
            name: path(&["tool"]),
            key: id("description"),
            value: StrLit::one_line(description),
        }],
        public: false,
        is_async: true,
        name: id(&method_name),
        generics: vec![],
        params: vec![Param::SelfRef, parameters_param],
        params_layout: Params::Inline,
        ret: TypeExpr::Ir(RustType::String),
        body: Block(body),
    }
}

/// `let req = crate::api::types::XRequest { field: input.field.clone(), … };`
fn request_body_stmt(op: &Operation) -> Stmt {
    let fields = op
        .request_body
        .as_ref()
        .map(|b| {
            b.fields
                .iter()
                .map(|f| {
                    FieldInit::Named(
                        id(&f.rust_name),
                        call(input_field(&f.rust_name), "clone", vec![]),
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    Stmt::Let {
        name: id("req"),
        mutable: false,
        ty: None,
        init: Expr::StructLit {
            path: path(&["crate", "api", "types", &op.request_body_type_name()]),
            fields,
            braces: Braces::Multiline,
        },
    }
}

// ── Test shims ─────────────────────────────────────────────────────────────
//
// These preserve the pre-refactor unit tests unchanged while pointing them at
// the typed surface that replaced the hand-rolled helpers.

/// The rendered field type, e.g. `Option<String>`.
#[cfg(test)]
fn input_field_type(rt: &RustType, required: bool) -> String {
    input_field_ty(rt, required).to_rust()
}

#[cfg(test)]
fn rust_type_string(rt: &RustType) -> String {
    TypeExpr::Ir(rt.clone()).to_rust()
}

#[cfg(test)]
fn is_option_type(rt: &RustType) -> bool {
    rt.is_option()
}

/// The escaped body of a single-line string literal, quotes stripped.
///
/// Escaping now lives in `StrLit`; this renders through it so the original
/// escaping tests still assert against the code that actually runs.
#[cfg(test)]
fn escape_string(s: &str) -> String {
    let quoted = StrLit::one_line(s).to_rust();
    quoted[1..quoted.len() - 1].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ApiSpec, AuthMethod, FieldDef, OpRequestBody};

    fn make_spec(operations: Vec<Operation>) -> ApiSpec {
        ApiSpec {
            name: "TestApi".into(),
            description: Some("Test API for unit tests.".into()),
            version: "1.0.0".into(),
            base_url: Some("https://api.example.com".into()),
            auth: AuthMethod::Bearer,
            operations,
            types: vec![],
        }
    }

    fn make_get_op(id: &str, path: &str) -> Operation {
        Operation {
            id: id.into(),
            method: HttpMethod::Get,
            path: path.into(),
            summary: Some(format!("Get {id}")),
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        }
    }

    // -- Top-level structure --

    #[test]
    fn generates_rmcp_imports() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("use rmcp::"));
        assert!(code.contains("ServerHandler"));
        assert!(code.contains("ToolRouter"));
    }

    #[test]
    fn generates_mcp_struct() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("struct TestApiMcp {"));
        assert!(code.contains("client: TestApiClient,"));
        assert!(code.contains("tool_router: ToolRouter<Self>,"));
    }

    #[test]
    fn generates_tool_router_annotation() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[tool_router]"));
    }

    #[test]
    fn generates_server_handler_impl() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[tool_handler]"));
        assert!(code.contains("impl ServerHandler for TestApiMcp {"));
        assert!(code.contains("fn get_info(&self) -> ServerInfo {"));
    }

    #[test]
    fn generates_server_instructions() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("Test API for unit tests."));
    }

    #[test]
    fn generates_entry_point() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn run()"));
        assert!(code.contains("TestApiMcp::new()?"));
        assert!(code.contains("serve(stdio())"));
    }

    #[test]
    fn generates_constructor_with_config() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("fn new() -> Result<Self, String>"));
        assert!(code.contains("TestApiConfig::load()"));
        assert!(code.contains("auth::resolve_api_key"));
        assert!(code.contains("TestApiClient::new"));
    }

    // -- Input structs --

    #[test]
    fn generates_input_struct_for_parameterized_op() {
        let op = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{id}".into(),
            summary: Some("Get an item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: Some("The item ID".into()),
            }],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("struct GetItemInput {"));
        assert!(code.contains("id: String,"));
        assert!(code.contains("#[schemars(description = \"The item ID\")]"));
    }

    #[test]
    fn skips_empty_input_struct() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        // No ListItemsInput struct should be generated
        assert!(!code.contains("ListItemsInput {"));
        // But the tool method should still exist with serde_json::Value
        assert!(code.contains("Parameters<serde_json::Value>"));
    }

    #[test]
    fn generates_input_struct_with_body_fields() {
        let op = Operation {
            id: "create_item".into(),
            method: HttpMethod::Post,
            path: "/items".into(),
            summary: Some("Create an item".into()),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "name".into(),
                    rust_name: "name".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: Some("Item name".into()),
                    default_value: None,
                }],
                type_name: Some("CreateItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("struct CreateItemInput {"));
        assert!(code.contains("name: String,"));
    }

    // -- Tool methods --

    #[test]
    fn generates_tool_annotation_with_description() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("#[tool(description = \"Get list_items\")]"));
    }

    #[test]
    fn generates_tool_method_for_get() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("async fn list_items("));
        assert!(code.contains("match self.client.list_items("));
        assert!(code.contains("format::format_list_items(&result)"));
    }

    #[test]
    fn delete_operations_use_simple_success_message() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: Some("Delete an item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("Ok(_) => format!(\"Success: DELETE /items/{id}\")"));
    }

    #[test]
    fn generates_error_handling() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        // A failed call is a typed BLIND answer, not English prose — the
        // reader is a model with only the bytes, and "Error: connection
        // refused" is a sentence it must parse rather than a discriminant
        // it can branch on.
        assert!(code.contains("Err(e) => kotae::Answer::blind(e.to_string()).render()"));
        assert!(
            !code.contains("format!(\"Error:"),
            "no generated tool may answer a failure with prose",
        );
    }

    #[test]
    fn generates_request_body_construction() {
        let op = Operation {
            id: "create_item".into(),
            method: HttpMethod::Post,
            path: "/items".into(),
            summary: Some("Create an item".into()),
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
                        description: None,
                        default_value: None,
                    },
                    FieldDef {
                        name: "count".into(),
                        rust_name: "count".into(),
                        rust_type: RustType::I64,
                        required: true,
                        description: None,
                        default_value: None,
                    },
                ],
                type_name: Some("CreateItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("let req = crate::api::types::CreateItemRequest {"));
        assert!(code.contains("name: input.name.clone(),"));
        assert!(code.contains("count: input.count.clone(),"));
        assert!(code.contains("&req"));
    }

    // -- Helper function tests --

    #[test]
    fn escape_string_handles_quotes() {
        assert_eq!(escape_string("say \"hello\""), "say \\\"hello\\\"");
    }

    #[test]
    fn escape_string_handles_backslashes() {
        assert_eq!(escape_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_string_handles_newlines() {
        assert_eq!(escape_string("line1\nline2"), "line1 line2");
    }

    #[test]
    fn escape_string_no_change_for_simple() {
        assert_eq!(escape_string("simple text"), "simple text");
    }

    #[test]
    fn input_field_type_required_string() {
        assert_eq!(input_field_type(&RustType::String, true), "String");
    }

    #[test]
    fn input_field_type_optional_wraps() {
        assert_eq!(
            input_field_type(&RustType::String, false),
            "Option<String>"
        );
    }

    #[test]
    fn input_field_type_already_option_not_double_wrapped() {
        assert_eq!(
            input_field_type(&RustType::Option(Box::new(RustType::String)), false),
            "Option<String>"
        );
    }

    #[test]
    fn rust_type_string_all_variants() {
        assert_eq!(rust_type_string(&RustType::String), "String");
        assert_eq!(rust_type_string(&RustType::I64), "i64");
        assert_eq!(rust_type_string(&RustType::U64), "u64");
        assert_eq!(rust_type_string(&RustType::F64), "f64");
        assert_eq!(rust_type_string(&RustType::Bool), "bool");
        assert_eq!(rust_type_string(&RustType::Value), "serde_json::Value");
        assert_eq!(
            rust_type_string(&RustType::Vec(Box::new(RustType::I64))),
            "Vec<i64>"
        );
        assert_eq!(
            rust_type_string(&RustType::Named("Foo".into())),
            "Foo"
        );
    }

    #[test]
    fn is_option_type_true() {
        assert!(is_option_type(&RustType::Option(Box::new(RustType::String))));
    }

    #[test]
    fn is_option_type_false() {
        assert!(!is_option_type(&RustType::String));
        assert!(!is_option_type(&RustType::Vec(Box::new(RustType::String))));
    }

    #[test]
    fn default_description_fallback() {
        let op = Operation {
            id: "do_thing".into(),
            method: HttpMethod::Post,
            path: "/thing".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        // Fallback description: "do_thing operation"
        assert!(code.contains("do_thing operation"));
    }

    #[test]
    fn default_instructions_fallback() {
        let mut spec = make_spec(vec![]);
        spec.description = None;
        let code = generate(&spec);
        assert!(code.contains("TestApi MCP server"));
    }

    // -- Description fallback to operation description when no summary --

    #[test]
    fn tool_description_from_description_field() {
        let op = Operation {
            id: "do_thing".into(),
            method: HttpMethod::Post,
            path: "/thing".into(),
            summary: None,
            description: Some("A long description for this operation".into()),
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("A long description for this operation"));
    }

    // -- Mixed path + query + body params --

    #[test]
    fn tool_with_mixed_params() {
        let op = Operation {
            id: "update_user_setting".into(),
            method: HttpMethod::Put,
            path: "/users/{userId}/settings".into(),
            summary: Some("Update a user setting".into()),
            description: None,
            parameters: vec![
                OpParameter {
                    name: "userId".into(),
                    rust_name: "user_id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: Some("User identifier".into()),
                },
                OpParameter {
                    name: "force".into(),
                    rust_name: "force".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::Bool)),
                    description: None,
                },
            ],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "value".into(),
                    rust_name: "value".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: Some("Setting value".into()),
                    default_value: None,
                }],
                type_name: Some("UpdateSettingRequest".into()),
            }),
            response_type: Some(RustType::Named("Setting".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("struct UpdateUserSettingInput {"));
        assert!(code.contains("user_id: String,"));
        assert!(code.contains("force: Option<bool>,"));
        assert!(code.contains("value: String,"));
        assert!(code.contains("&input.user_id"));
    }

    // -- Query option in tool method uses as_deref --

    #[test]
    fn query_option_uses_as_deref_in_tool() {
        let op = Operation {
            id: "search".into(),
            method: HttpMethod::Get,
            path: "/search".into(),
            summary: Some("Search".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "q".into(),
                rust_name: "q".into(),
                location: ParamLocation::Query,
                required: false,
                rust_type: RustType::Option(Box::new(RustType::String)),
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("input.q.as_deref()"),
            "Option query param should use as_deref in tool method"
        );
    }

    // -- Non-option query param passed directly --

    #[test]
    fn query_non_option_passed_directly_in_tool() {
        let op = Operation {
            id: "list_items".into(),
            method: HttpMethod::Get,
            path: "/items".into(),
            summary: Some("List items".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "limit".into(),
                rust_name: "limit".into(),
                location: ParamLocation::Query,
                required: true,
                rust_type: RustType::I64,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("input.limit"),
            "required query param should be passed directly"
        );
    }

    // -- Header params are NOT included in MCP input struct --

    #[test]
    fn header_params_excluded_from_input_struct() {
        let op = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{id}".into(),
            summary: Some("Get item".into()),
            description: None,
            parameters: vec![
                OpParameter {
                    name: "id".into(),
                    rust_name: "id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: None,
                },
                OpParameter {
                    name: "X-Request-Id".into(),
                    rust_name: "x_request_id".into(),
                    location: ParamLocation::Header,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::String)),
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("id: String,"));
        assert!(
            !code.contains("x_request_id"),
            "header params should not appear in MCP input struct"
        );
    }

    // -- Escape string handles combined special chars --

    #[test]
    fn escape_string_combined() {
        assert_eq!(
            escape_string("say \"hello\"\nand\\goodbye"),
            "say \\\"hello\\\" and\\\\goodbye"
        );
    }

    // -- input_field_type with Vec --

    #[test]
    fn input_field_type_vec() {
        assert_eq!(
            input_field_type(&RustType::Vec(Box::new(RustType::String)), true),
            "Vec<String>"
        );
        assert_eq!(
            input_field_type(&RustType::Vec(Box::new(RustType::String)), false),
            "Option<Vec<String>>"
        );
    }

    // -- MCP struct derives Debug and Clone --

    #[test]
    fn mcp_struct_has_derive() {
        let spec = make_spec(vec![]);
        let code = generate(&spec);
        assert!(code.contains("#[derive(Debug, Clone)]"));
    }

    // -- Multiple operations generate multiple tools --

    #[test]
    fn multiple_operations_generate_multiple_tools() {
        let op1 = make_get_op("list_items", "/items");
        let op2 = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{id}".into(),
            summary: Some("Get item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op1, op2]);
        let code = generate(&spec);
        assert!(code.contains("async fn list_items("));
        assert!(code.contains("async fn get_item("));
    }

    // -- stop_ prefix uses simple success message --

    #[test]
    fn stop_operations_use_simple_success_message() {
        let op = Operation {
            id: "stop_service".into(),
            method: HttpMethod::Post,
            path: "/services/{id}/stop".into(),
            summary: Some("Stop a service".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("Ok(_) => format!(\"Success:"),
            "stop_ prefixed operations should use simple success message"
        );
    }

    // -- Request body construction with no type_name uses fallback --

    #[test]
    fn request_body_construction_fallback_name() {
        let op = Operation {
            id: "send_data".into(),
            method: HttpMethod::Post,
            path: "/data".into(),
            summary: Some("Send data".into()),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "payload".into(),
                    rust_name: "payload".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: None,
                    default_value: None,
                }],
                type_name: None,
            }),
            response_type: Some(RustType::Named("Result".into())),
            errors: vec![],
        };
        let spec = make_spec(vec![op]);
        let code = generate(&spec);
        assert!(code.contains("SendDataRequest"));
    }
}
