//! Emit `src/client.rs` — the typed HTTP client — from the API spec.
//!
//! Every construct is built as a [`rust_ast`] tree and rendered once. Per
//! ★★ TYPED EMISSION there is no `format!()` of Rust syntax here; the only
//! `format!` calls left build *names* (`{pascal}Client`) and a user-agent
//! string, neither of which is syntax.

use super::rust_ast::{
    render_file, Attr, Block, Braces, ChainLink, Doc, Expr, FieldDecl, FieldInit, FnDef,
    FormatTemplate, GenericParam, Ident, ImplBlock, ImplItem, Item, Param, Params, Path, Stmt,
    StructDef, TypeExpr,
};
use crate::ir::{ApiSpec, AuthMethod, HttpMethod, OpParameter, Operation, ParamLocation, RustType};
use heck::{ToSnakeCase, ToUpperCamelCase};

// ── Identifier helpers ─────────────────────────────────────────────────────
//
// Names reaching the emitter come from the IR, which has already normalised
// them (`rust_name`, `to_snake_case`). `Ident::new` is the parse boundary: a
// name that is not a legal Rust identifier stops generation here rather than
// producing a file that will not compile.

/// # Panics
///
/// Panics if `name` is not a legal Rust identifier. That indicates an IR bug
/// — the IR is responsible for normalising names — and there is no sane
/// partial output to emit, so generation stops.
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

/// `receiver.method(args)`
fn call(receiver: Expr, method: &str, args: Vec<Expr>) -> Expr {
    Expr::MethodCall(Box::new(receiver), id(method), args)
}

/// `Result<T>`
fn result_of(inner: TypeExpr) -> TypeExpr {
    TypeExpr::App(path(&["Result"]), vec![inner])
}

/// `let <name> = <init>;`
fn let_stmt(name: &str, init: Expr) -> Stmt {
    Stmt::Let { name: id(name), mutable: false, ty: None, init }
}

// ── Entry point ────────────────────────────────────────────────────────────

/// Generate the `src/client.rs` file from the API spec.
///
/// Produces a typed HTTP client struct with:
/// - Auth based on `spec.auth` (Bearer, Basic, `ApiKeyHeader`, None)
/// - Typed async methods for each operation
/// - Path parameter interpolation and query parameter URL-encoding
#[must_use]
pub fn generate(spec: &ApiSpec) -> String {
    let pascal = spec.name.to_upper_camel_case();
    let client_name = format!("{pascal}Client");
    let error_name = format!("{pascal}Error");
    let error = path(&[&error_name]);

    let mut doc = vec![format!("HTTP client for the {} API.", spec.name)];
    if let Some(desc) = &spec.description {
        doc.push(String::new());
        doc.push(desc.clone());
    }

    let client_struct = StructDef {
        doc: Doc(doc),
        attrs: vec![Attr::List(id("derive"), vec![path(&["Debug"]), path(&["Clone"])])],
        public: true,
        name: id(&client_name),
        fields: vec![
            field("inner", TypeExpr::Path(path(&["reqwest", "Client"]))),
            field("base_url", TypeExpr::Ir(RustType::String)),
            field("api_key", TypeExpr::Ir(RustType::String)),
        ],
    };

    let mut members = vec![
        ImplItem::Fn(constructor(spec, &error)),
        ImplItem::Blank,
        ImplItem::Fn(url_helper()),
        ImplItem::Blank,
    ];
    for helper in HTTP_HELPERS {
        members.push(ImplItem::Fn(http_helper(helper, &spec.auth, &error)));
        members.push(ImplItem::Blank);
    }
    members.push(ImplItem::Fn(handle_response(&error)));
    members.push(ImplItem::Blank);
    members.push(ImplItem::Fn(handle_empty_response(&error)));
    members.push(ImplItem::Blank);
    members.push(ImplItem::Comment("-- Public API methods --".into()));
    members.push(ImplItem::Blank);
    for op in &spec.operations {
        members.push(ImplItem::Fn(operation_method(op)));
        members.push(ImplItem::Blank);
    }

    let items = vec![
        Item::Use { path: path(&["crate", "api", "types"]), leaves: vec![], glob: true },
        Item::Use {
            path: path(&["crate", "error"]),
            leaves: vec![error.clone(), path(&["Result"])],
            glob: false,
        },
        Item::Blank,
        Item::Struct(client_struct),
        Item::Blank,
        Item::Impl(ImplBlock {
            attrs: vec![],
            trait_path: None,
            self_ty: path(&[&client_name]),
            items: members,
        }),
    ];

    render_file(&items)
}

fn field(name: &str, ty: TypeExpr) -> FieldDecl {
    FieldDecl { attrs: vec![], public: false, name: id(name), ty }
}

// ── Constructor and helpers ────────────────────────────────────────────────

fn constructor(spec: &ApiSpec, error: &Path) -> FnDef {
    let user_agent = format!("pleme-io/{} {}", spec.name.to_snake_case(), spec.version);

    // reqwest::Client::builder()
    //     .timeout(std::time::Duration::from_secs(60))
    //     .user_agent("…")
    //     .build()
    //     .map_err(<Error>::Request)?
    let builder = Expr::Try(Box::new(Expr::Chain {
        receiver: Box::new(Expr::Call(path(&["reqwest", "Client", "builder"]), vec![])),
        links: vec![
            ChainLink::Call(
                id("timeout"),
                vec![Expr::Call(
                    path(&["std", "time", "Duration", "from_secs"]),
                    vec![Expr::Int(60)],
                )],
            ),
            ChainLink::Call(id("user_agent"), vec![Expr::string(user_agent)]),
            ChainLink::Call(id("build"), vec![]),
            ChainLink::Call(
                id("map_err"),
                vec![Expr::Path(error.clone().join(id("Request")))],
            ),
        ],
    }));

    let ok_self = Expr::Call(
        path(&["Ok"]),
        vec![Expr::StructLit {
            path: path(&["Self"]),
            fields: vec![
                FieldInit::Shorthand(id("inner")),
                FieldInit::Named(
                    id("base_url"),
                    call(
                        call(var("base_url"), "trim_end_matches", vec![Expr::Char('/')]),
                        "to_string",
                        vec![],
                    ),
                ),
                FieldInit::Named(id("api_key"), call(var("api_key"), "to_string", vec![])),
            ],
            braces: Braces::Multiline,
        }],
    );

    FnDef {
        doc: Doc(vec!["Create a new client.".into()]),
        attrs: vec![],
        public: true,
        is_async: false,
        name: id("new"),
        generics: vec![],
        params: vec![
            Param::Typed(id("base_url"), TypeExpr::str_ref()),
            Param::Typed(id("api_key"), TypeExpr::str_ref()),
        ],
        params_layout: Params::Inline,
        ret: result_of(TypeExpr::Path(path(&["Self"]))),
        body: Block(vec![
            let_stmt("inner", builder),
            Stmt::Blank,
            Stmt::Tail(ok_self),
        ]),
    }
}

fn url_helper() -> FnDef {
    // format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    let body = Expr::Format(
        FormatTemplate::new().hole().lit("/").hole(),
        vec![
            Expr::Field(Box::new(var("self")), id("base_url")),
            call(var("path"), "trim_start_matches", vec![Expr::Char('/')]),
        ],
    );

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: false,
        is_async: false,
        name: id("url"),
        generics: vec![],
        params: vec![Param::SelfRef, Param::Typed(id("path"), TypeExpr::str_ref())],
        params_layout: Params::Inline,
        ret: TypeExpr::Ir(RustType::String),
        body: Block(vec![Stmt::Tail(body)]),
    }
}

// ── The HTTP helper table ──────────────────────────────────────────────────
//
// Twelve helpers that differ only in verb, whether they take a request body,
// and whether they decode a response. They used to be twelve near-identical
// blocks of string concatenation; they are now twelve rows and one builder.
// Adding a helper is a row.

/// One row of the private-helper table.
struct Helper {
    name: &'static str,
    verb: HttpMethod,
    /// Takes a `body: &B` parameter serialised with `.json(body)`.
    has_body: bool,
    /// Decodes a response into `T`; otherwise returns `Result<()>`.
    returns_value: bool,
}

const HTTP_HELPERS: &[Helper] = &[
    Helper { name: "get", verb: HttpMethod::Get, has_body: false, returns_value: true },
    Helper { name: "post", verb: HttpMethod::Post, has_body: true, returns_value: true },
    Helper { name: "post_empty", verb: HttpMethod::Post, has_body: false, returns_value: true },
    Helper { name: "put", verb: HttpMethod::Put, has_body: true, returns_value: true },
    Helper { name: "patch", verb: HttpMethod::Patch, has_body: true, returns_value: true },
    Helper { name: "delete", verb: HttpMethod::Delete, has_body: false, returns_value: true },
    Helper { name: "delete_empty", verb: HttpMethod::Delete, has_body: false, returns_value: false },
    Helper {
        name: "post_empty_no_response",
        verb: HttpMethod::Post,
        has_body: false,
        returns_value: false,
    },
    Helper { name: "post_no_response", verb: HttpMethod::Post, has_body: true, returns_value: false },
    Helper { name: "put_no_response", verb: HttpMethod::Put, has_body: true, returns_value: false },
    Helper {
        name: "patch_no_response",
        verb: HttpMethod::Patch,
        has_body: true,
        returns_value: false,
    },
    Helper { name: "get_empty", verb: HttpMethod::Get, has_body: false, returns_value: false },
];

/// The reqwest method for an HTTP verb.
fn verb_method(verb: HttpMethod) -> &'static str {
    match verb {
        HttpMethod::Get => "get",
        HttpMethod::Post => "post",
        HttpMethod::Put => "put",
        HttpMethod::Patch => "patch",
        HttpMethod::Delete => "delete",
    }
}

/// The authentication link in the request chain.
///
/// [`AuthMethod::None`] yields [`ChainLink::Absent`]: no call, but the line it
/// occupied is retained, matching the established output.
fn auth_link(auth: &AuthMethod) -> ChainLink {
    match auth {
        AuthMethod::Bearer => ChainLink::Call(
            id("bearer_auth"),
            vec![Expr::Ref(Box::new(Expr::Field(Box::new(var("self")), id("api_key"))))],
        ),
        AuthMethod::Basic => ChainLink::Call(
            id("basic_auth"),
            vec![
                Expr::Ref(Box::new(Expr::Field(Box::new(var("self")), id("api_key")))),
                Expr::Turbofish {
                    base: path(&["Option"]),
                    ty: Box::new(TypeExpr::str_ref()),
                    member: id("None"),
                },
            ],
        ),
        AuthMethod::ApiKeyHeader(header) => ChainLink::Call(
            id("header"),
            vec![
                Expr::string(header.clone()),
                Expr::Ref(Box::new(Expr::Field(Box::new(var("self")), id("api_key")))),
            ],
        ),
        AuthMethod::None => ChainLink::Absent,
    }
}

fn http_helper(h: &Helper, auth: &AuthMethod, error: &Path) -> FnDef {
    let mut generics = Vec::new();
    if h.has_body {
        generics.push(GenericParam {
            name: id("B"),
            bound: Some(path(&["serde", "Serialize"])),
        });
    }
    if h.returns_value {
        generics.push(GenericParam {
            name: id("T"),
            bound: Some(path(&["serde", "de", "DeserializeOwned"])),
        });
    }

    let mut params = vec![Param::SelfRef, Param::Typed(id("path"), TypeExpr::str_ref())];
    if h.has_body {
        params.push(Param::Typed(
            id("body"),
            TypeExpr::Ref(Box::new(TypeExpr::Path(path(&["B"])))),
        ));
    }

    let mut links = vec![
        ChainLink::Field(id("inner")),
        ChainLink::Call(
            id(verb_method(h.verb)),
            vec![Expr::Ref(Box::new(call(var("self"), "url", vec![var("path")])))],
        ),
        auth_link(auth),
    ];
    if h.has_body {
        links.push(ChainLink::Call(id("json"), vec![var("body")]));
    }
    links.push(ChainLink::Call(id("send"), vec![]));
    links.push(ChainLink::Await);
    links.push(ChainLink::Call(
        id("map_err"),
        vec![Expr::Path(error.clone().join(id("Request")))],
    ));

    let handler = if h.returns_value { "handle_response" } else { "handle_empty_response" };

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: false,
        is_async: true,
        name: id(h.name),
        generics,
        params,
        params_layout: if h.has_body { Params::OnePerLine } else { Params::Inline },
        ret: result_of(if h.returns_value {
            TypeExpr::Path(path(&["T"]))
        } else {
            TypeExpr::Unit
        }),
        body: Block(vec![
            let_stmt(
                "resp",
                Expr::Try(Box::new(Expr::Chain {
                    receiver: Box::new(var("self")),
                    links,
                })),
            ),
            Stmt::Tail(Expr::Await(Box::new(Expr::Call(
                path(&["Self", handler]),
                vec![var("resp")],
            )))),
        ]),
    }
}

/// The shared prelude of both response handlers: read the status, and bail
/// with the body text when it is not a success.
fn status_guard(error: &Path) -> Vec<Stmt> {
    vec![
        let_stmt("status", call(call(var("resp"), "status", vec![]), "as_u16", vec![])),
        Stmt::If {
            cond: Expr::Not(Box::new(call(
                call(var("resp"), "status", vec![]),
                "is_success",
                vec![],
            ))),
            then: Block(vec![
                let_stmt(
                    "body",
                    call(
                        Expr::Await(Box::new(call(var("resp"), "text", vec![]))),
                        "unwrap_or_default",
                        vec![],
                    ),
                ),
                Stmt::Return(Expr::Call(
                    path(&["Err"]),
                    vec![Expr::StructLit {
                        path: error.clone().join(id("Api")),
                        fields: vec![
                            FieldInit::Shorthand(id("status")),
                            FieldInit::Shorthand(id("body")),
                        ],
                        braces: Braces::Inline,
                    }],
                )),
            ]),
        },
    ]
}

fn handle_response(error: &Path) -> FnDef {
    let mut body = status_guard(error);
    body.push(let_stmt(
        "text",
        Expr::Try(Box::new(call(
            Expr::Await(Box::new(call(var("resp"), "text", vec![]))),
            "map_err",
            vec![Expr::Path(error.clone().join(id("Request")))],
        ))),
    ));
    body.push(Stmt::Tail(call(
        Expr::Call(path(&["serde_json", "from_str"]), vec![Expr::Ref(Box::new(var("text")))]),
        "map_err",
        vec![Expr::Path(error.clone().join(id("Json")))],
    )));

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: false,
        is_async: true,
        name: id("handle_response"),
        generics: vec![GenericParam {
            name: id("T"),
            bound: Some(path(&["serde", "de", "DeserializeOwned"])),
        }],
        params: vec![Param::Typed(
            id("resp"),
            TypeExpr::Path(path(&["reqwest", "Response"])),
        )],
        params_layout: Params::OnePerLine,
        ret: result_of(TypeExpr::Path(path(&["T"]))),
        body: Block(body),
    }
}

fn handle_empty_response(error: &Path) -> FnDef {
    let mut body = status_guard(error);
    body.push(Stmt::Tail(Expr::Call(path(&["Ok"]), vec![Expr::Unit])));

    FnDef {
        doc: Doc::default(),
        attrs: vec![],
        public: false,
        is_async: true,
        name: id("handle_empty_response"),
        generics: vec![],
        params: vec![Param::Typed(
            id("resp"),
            TypeExpr::Path(path(&["reqwest", "Response"])),
        )],
        params_layout: Params::OnePerLine,
        ret: result_of(TypeExpr::Unit),
        body: Block(body),
    }
}

// ── Public operation methods ───────────────────────────────────────────────

/// An OpenAPI path split into literal text and `{param}` holes.
///
/// Parsing the path once, into typed segments, is what lets the emitter build
/// a `format!` template with real holes instead of running `str::replace` over
/// syntax and hoping the braces line up.
fn path_template(op_path: &str, path_params: &[&OpParameter]) -> FormatTemplate {
    let mut template = FormatTemplate::new();
    let mut rest = op_path;

    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open..].find('}') else {
            break;
        };
        let close = open + close_rel;
        let name = &rest[open + 1..close];

        if !rest[..open].is_empty() {
            template = template.lit(&rest[..open]);
        }
        match path_params.iter().find(|p| p.name == name) {
            Some(param) => template = template.captured(id(&param.rust_name)),
            // Not a declared parameter: emit the braces literally, as the
            // renderer will double them.
            None => template = template.lit(&rest[open..=close]),
        }
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        template = template.lit(rest);
    }
    template
}

#[allow(clippy::too_many_lines)]
fn operation_method(op: &Operation) -> FnDef {
    let path_params: Vec<&OpParameter> =
        op.parameters.iter().filter(|p| p.location == ParamLocation::Path).collect();
    let query_params: Vec<&OpParameter> =
        op.parameters.iter().filter(|p| p.location == ParamLocation::Query).collect();

    let has_body = op.request_body.is_some();
    let has_response = op.response_type.is_some();

    let mut doc = vec![format!("{} {}", op.method, op.path)];
    if let Some(summary) = &op.summary {
        doc.push(String::new());
        doc.push(summary.clone());
    }

    let mut params = vec![Param::SelfRef];
    for p in &path_params {
        params.push(Param::Typed(id(&p.rust_name), TypeExpr::str_ref()));
    }
    for p in &query_params {
        params.push(Param::Typed(id(&p.rust_name), param_type(p)));
    }
    if has_body {
        params.push(Param::Typed(
            id("req"),
            TypeExpr::Ref(Box::new(TypeExpr::Path(path(&[&op.request_body_type_name()])))),
        ));
    }

    let helper = if has_response {
        http_method_fn(op.method, has_body)
    } else {
        http_method_fn_empty(op.method, has_body)
    };

    // The call to the private helper, given the expression for its path.
    let invoke = |path_expr: Expr| {
        let mut args = vec![path_expr];
        if has_body {
            args.push(var("req"));
        }
        Stmt::Tail(Expr::Await(Box::new(call(var("self"), helper, args))))
    };

    let body = if path_params.is_empty() && query_params.is_empty() {
        // A static path needs no interpolation.
        vec![invoke(Expr::string(op.path.clone()))]
    } else {
        let template = path_template(&op.path, &path_params);

        if query_params.is_empty() {
            vec![invoke(Expr::Ref(Box::new(Expr::Format(template, vec![]))))]
        } else {
            let mut stmts = vec![
                Stmt::Let {
                    name: id("path"),
                    mutable: true,
                    ty: None,
                    init: Expr::Format(template, vec![]),
                },
                Stmt::Let {
                    name: id("has_query"),
                    mutable: true,
                    ty: None,
                    init: Expr::Bool(false),
                },
            ];
            for p in &query_params {
                stmts.extend(query_param_stmts(p));
            }
            stmts.push(invoke(Expr::Ref(Box::new(var("path")))));
            stmts
        }
    };

    FnDef {
        doc: Doc(doc),
        attrs: vec![],
        public: true,
        is_async: true,
        name: id(&op.id.to_snake_case()),
        generics: vec![],
        params,
        params_layout: Params::OnePerLine,
        ret: result_of(
            op.response_type.as_ref().map_or(TypeExpr::Unit, |rt| TypeExpr::Ir(rt.clone())),
        ),
        body: Block(body),
    }
}

/// The statements that append one query parameter to `path`.
///
/// The separator is chosen at runtime from `has_query` so it stays correct
/// whichever optional parameters happen to be present.
fn query_param_stmts(p: &OpParameter) -> Vec<Stmt> {
    let separator = Stmt::Semi(call(
        var("path"),
        "push_str",
        vec![Expr::IfElseInline {
            cond: Box::new(var("has_query")),
            then: Box::new(Expr::string("&")),
            otherwise: Box::new(Expr::string("?")),
        }],
    ));

    // path.push_str(&format!("<name>={}", urlencoding::encode(&<value>.to_string())));
    let append = |value: Expr| {
        Stmt::Semi(call(
            var("path"),
            "push_str",
            vec![Expr::Ref(Box::new(Expr::Format(
                FormatTemplate::new().lit(&p.name).lit("=").hole(),
                vec![Expr::Call(
                    path(&["urlencoding", "encode"]),
                    vec![Expr::Ref(Box::new(call(value, "to_string", vec![])))],
                )],
            )))],
        ))
    };

    let set_flag = Stmt::Assign { lhs: var("has_query"), rhs: Expr::Bool(true) };

    if p.rust_type.is_option() {
        vec![Stmt::IfLetSomeRef {
            name: id("v"),
            scrutinee: var(&p.rust_name),
            then: Block(vec![separator, append(var("v")), set_flag]),
        }]
    } else {
        vec![separator, append(var(&p.rust_name)), set_flag]
    }
}

/// Select the private helper method name for operations that return a response body.
fn http_method_fn(method: HttpMethod, has_body: bool) -> &'static str {
    match method {
        HttpMethod::Get => "get",
        HttpMethod::Post => {
            if has_body {
                "post"
            } else {
                "post_empty"
            }
        }
        HttpMethod::Put => "put",
        HttpMethod::Patch => "patch",
        HttpMethod::Delete => "delete",
    }
}

/// Select the private helper method name for operations with no response body (e.g. 204).
fn http_method_fn_empty(method: HttpMethod, has_body: bool) -> &'static str {
    match method {
        HttpMethod::Get => "get_empty",
        HttpMethod::Post => {
            if has_body {
                "post_no_response"
            } else {
                "post_empty_no_response"
            }
        }
        HttpMethod::Put => "put_no_response",
        HttpMethod::Patch => "patch_no_response",
        HttpMethod::Delete => "delete_empty",
    }
}

/// The parameter type of a query parameter in the public method signature.
fn param_type(param: &OpParameter) -> TypeExpr {
    if param.required {
        match &param.rust_type {
            RustType::String => TypeExpr::str_ref(),
            RustType::Option(inner) => TypeExpr::Ir((**inner).clone()),
            other => TypeExpr::Ir(other.clone()),
        }
    } else {
        match &param.rust_type {
            RustType::Option(_) => TypeExpr::Ir(param.rust_type.clone()),
            other => TypeExpr::App(path(&["Option"]), vec![TypeExpr::Ir(other.clone())]),
        }
    }
}

#[cfg(test)]
fn param_type_string(param: &OpParameter) -> String {
    param_type(param).to_rust()
}

#[cfg(test)]
fn request_body_type_name(op: &Operation) -> String {
    op.request_body_type_name()
}

#[cfg(test)]
fn is_option_type(rt: &RustType) -> bool {
    rt.is_option()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{ApiSpec, AuthMethod, FieldDef, OpRequestBody};

    fn make_spec(
        name: &str,
        auth: AuthMethod,
        operations: Vec<Operation>,
    ) -> ApiSpec {
        ApiSpec {
            name: name.into(),
            description: None,
            version: "1.0.0".into(),
            base_url: Some("https://api.example.com".into()),
            auth,
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

    fn make_post_op(id: &str, path: &str) -> Operation {
        Operation {
            id: id.into(),
            method: HttpMethod::Post,
            path: path.into(),
            summary: Some(format!("Create {id}")),
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "name".into(),
                    rust_name: "name".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: None,
                    default_value: None,
                }],
                type_name: Some("CreateItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        }
    }

    /// Render the auth chain link the way the emitter does, for assertions
    /// that used to compare against a hand-built string.
    fn auth_call(auth: &AuthMethod) -> String {
        match auth_link(auth) {
            ChainLink::Absent => String::new(),
            link => {
                let e = Expr::Chain { receiver: Box::new(var("x")), links: vec![link] };
                let f = FnDef {
                    doc: Doc::default(),
                    attrs: vec![],
                    public: false,
                    is_async: false,
                    name: id("f"),
                    generics: vec![],
                    params: vec![],
                    params_layout: Params::Inline,
                    ret: TypeExpr::Unit,
                    body: Block(vec![Stmt::Tail(e)]),
                };
                let text = render_file(&[Item::Fn(f)]);
                text.lines()
                    .find(|l| l.trim_start().starts_with('.'))
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default()
            }
        }
    }

    // -- Struct and constructor --

    #[test]
    fn generates_client_struct() {
        let spec = make_spec("TestApi", AuthMethod::Bearer, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub struct TestApiClient {"));
        assert!(code.contains("inner: reqwest::Client,"));
        assert!(code.contains("base_url: String,"));
        assert!(code.contains("api_key: String,"));
    }

    #[test]
    fn generates_constructor() {
        let spec = make_spec("TestApi", AuthMethod::Bearer, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub fn new(base_url: &str, api_key: &str)"));
        assert!(code.contains("reqwest::Client::builder()"));
        assert!(code.contains("timeout(std::time::Duration::from_secs(60))"));
    }

    #[test]
    fn generates_url_helper() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("fn url(&self, path: &str) -> String"));
    }

    #[test]
    fn generates_handle_response() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("async fn handle_response"));
        assert!(code.contains("is_success()"));
        assert!(code.contains("TestApiError::Api"));
    }

    // -- Auth methods --

    #[test]
    fn bearer_auth_call() {
        assert_eq!(
            auth_call(&AuthMethod::Bearer),
            ".bearer_auth(&self.api_key)"
        );
    }

    #[test]
    fn basic_auth_call() {
        assert_eq!(
            auth_call(&AuthMethod::Basic),
            ".basic_auth(&self.api_key, Option::<&str>::None)"
        );
    }

    #[test]
    fn api_key_header_auth_call() {
        assert_eq!(
            auth_call(&AuthMethod::ApiKeyHeader("X-Key".into())),
            ".header(\"X-Key\", &self.api_key)"
        );
    }

    #[test]
    fn no_auth_call() {
        assert_eq!(auth_call(&AuthMethod::None), "");
    }

    #[test]
    fn http_helpers_include_bearer_auth() {
        let spec = make_spec("TestApi", AuthMethod::Bearer, vec![]);
        let code = generate(&spec);
        assert!(code.contains(".bearer_auth(&self.api_key)"));
    }

    #[test]
    fn http_helpers_include_api_key_header() {
        let spec = make_spec("MyApi", AuthMethod::ApiKeyHeader("X-Api-Key".into()), vec![]);
        let code = generate(&spec);
        assert!(code.contains(".header(\"X-Api-Key\", &self.api_key)"));
    }

    // -- Operation methods --

    #[test]
    fn generates_get_method() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn list_items("));
        assert!(code.contains("-> Result<Item>"));
        assert!(code.contains("self.get(\"/items\").await"));
    }

    #[test]
    fn generates_post_method_with_body() {
        let op = make_post_op("create_item", "/items");
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn create_item("));
        assert!(code.contains("req: &CreateItemRequest,"));
        assert!(code.contains("self.post(\"/items\", req).await"));
    }

    #[test]
    fn generates_delete_method() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: None,
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
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn delete_item("));
        assert!(code.contains("id: &str,"));
        assert!(code.contains("self.delete("));
    }

    #[test]
    fn generates_path_parameter_interpolation() {
        let op = Operation {
            id: "get_item".into(),
            method: HttpMethod::Get,
            path: "/items/{itemId}".into(),
            summary: None,
            description: None,
            parameters: vec![OpParameter {
                name: "itemId".into(),
                rust_name: "item_id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: None,
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("item_id: &str,"));
        assert!(code.contains("format!(\"/items/{item_id}\")"));
    }

    #[test]
    fn generates_query_parameters() {
        let op = Operation {
            id: "list_items".into(),
            method: HttpMethod::Get,
            path: "/items".into(),
            summary: None,
            description: None,
            parameters: vec![
                OpParameter {
                    name: "limit".into(),
                    rust_name: "limit".into(),
                    location: ParamLocation::Query,
                    required: true,
                    rust_type: RustType::I64,
                    description: None,
                },
                OpParameter {
                    name: "cursor".into(),
                    rust_name: "cursor".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::String)),
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("limit: i64,"));
        assert!(code.contains("cursor: Option<String>,"));
        assert!(code.contains("urlencoding::encode"));
    }

    #[test]
    fn generates_doc_comment_for_operation() {
        let op = make_get_op("list_items", "/items");
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("/// GET /items"));
        assert!(code.contains("/// Get list_items"));
    }

    #[test]
    fn no_response_type_returns_unit() {
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
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("-> Result<()>"),
            "operations with no response_type should return Result<()>, got:\n{code}"
        );
        assert!(
            code.contains("self.post_empty_no_response("),
            "operations with no response_type should use the _no_response helper, got:\n{code}"
        );
    }

    // -- Helper function tests --

    #[test]
    fn http_method_fn_get() {
        assert_eq!(http_method_fn(HttpMethod::Get, false), "get");
    }

    #[test]
    fn http_method_fn_post_with_body() {
        assert_eq!(http_method_fn(HttpMethod::Post, true), "post");
    }

    #[test]
    fn http_method_fn_post_without_body() {
        assert_eq!(http_method_fn(HttpMethod::Post, false), "post_empty");
    }

    #[test]
    fn http_method_fn_put() {
        assert_eq!(http_method_fn(HttpMethod::Put, true), "put");
    }

    #[test]
    fn http_method_fn_patch() {
        assert_eq!(http_method_fn(HttpMethod::Patch, true), "patch");
    }

    #[test]
    fn http_method_fn_delete() {
        assert_eq!(http_method_fn(HttpMethod::Delete, false), "delete");
    }

    #[test]
    fn param_type_string_required_string() {
        let param = OpParameter {
            name: "name".into(),
            rust_name: "name".into(),
            location: ParamLocation::Query,
            required: true,
            rust_type: RustType::String,
            description: None,
        };
        assert_eq!(param_type_string(&param), "&str");
    }

    #[test]
    fn param_type_string_required_i64() {
        let param = OpParameter {
            name: "limit".into(),
            rust_name: "limit".into(),
            location: ParamLocation::Query,
            required: true,
            rust_type: RustType::I64,
            description: None,
        };
        assert_eq!(param_type_string(&param), "i64");
    }

    #[test]
    fn param_type_string_optional() {
        let param = OpParameter {
            name: "cursor".into(),
            rust_name: "cursor".into(),
            location: ParamLocation::Query,
            required: false,
            rust_type: RustType::Option(Box::new(RustType::String)),
            description: None,
        };
        assert_eq!(param_type_string(&param), "Option<String>");
    }

    #[test]
    fn request_body_type_name_from_type_name() {
        let op = make_post_op("create_item", "/items");
        assert_eq!(request_body_type_name(&op), "CreateItemRequest");
    }

    #[test]
    fn request_body_type_name_fallback() {
        let op = Operation {
            id: "update_item".into(),
            method: HttpMethod::Put,
            path: "/items".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![],
                type_name: None,
            }),
            response_type: None,
            errors: vec![],
        };
        assert_eq!(request_body_type_name(&op), "UpdateItemRequest");
    }

    #[test]
    fn client_description_included() {
        let mut spec = make_spec("TestApi", AuthMethod::None, vec![]);
        spec.description = Some("My test API description.".into());
        let code = generate(&spec);
        assert!(code.contains("My test API description."));
    }

    #[test]
    fn client_name_pascal_cased() {
        let spec = make_spec("my_api", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pub struct MyApiClient"));
    }

    // -- Bug fix: query parameter separator (runtime tracking) --

    #[test]
    fn query_params_use_runtime_separator_tracker() {
        // When optional param is first, followed by a required param, the
        // generated code must use a runtime `has_query` flag so the separator
        // is correct regardless of whether the optional param is present.
        let op = Operation {
            id: "list_items".into(),
            method: HttpMethod::Get,
            path: "/items".into(),
            summary: None,
            description: None,
            parameters: vec![
                OpParameter {
                    name: "status".into(),
                    rust_name: "status".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::String)),
                    description: None,
                },
                OpParameter {
                    name: "limit".into(),
                    rust_name: "limit".into(),
                    location: ParamLocation::Query,
                    required: true,
                    rust_type: RustType::I64,
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);

        // Must declare runtime tracker
        assert!(
            code.contains("let mut has_query = false;"),
            "generated code must declare runtime `has_query` tracker, got:\n{code}"
        );

        // Must NOT contain hard-coded '?' or '&' separator in format strings
        assert!(
            !code.contains("\"?status="),
            "generated code must not use compile-time '?' separator, got:\n{code}"
        );
        assert!(
            !code.contains("\"&limit="),
            "generated code must not use compile-time '&' separator, got:\n{code}"
        );

        // Must use runtime conditional separator
        assert!(
            code.contains("if has_query"),
            "generated code must check has_query at runtime, got:\n{code}"
        );
    }

    #[test]
    fn all_optional_query_params_use_runtime_separator() {
        // All query params are optional — each one must independently
        // check has_query at runtime.
        let op = Operation {
            id: "search".into(),
            method: HttpMethod::Get,
            path: "/search".into(),
            summary: None,
            description: None,
            parameters: vec![
                OpParameter {
                    name: "q".into(),
                    rust_name: "q".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::String)),
                    description: None,
                },
                OpParameter {
                    name: "page".into(),
                    rust_name: "page".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::I64)),
                    description: None,
                },
                OpParameter {
                    name: "per_page".into(),
                    rust_name: "per_page".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::I64)),
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);

        // Count occurrences of `has_query = true` — should be one per optional param
        let set_count = code.matches("has_query = true").count();
        assert_eq!(
            set_count, 3,
            "each optional query param should set has_query = true, found {set_count} times, got:\n{code}"
        );
    }

    // -- Bug fix: 204 No Content (empty response) --

    #[test]
    fn generates_handle_empty_response() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(
            code.contains("async fn handle_empty_response"),
            "must generate handle_empty_response helper, got:\n{code}"
        );
        assert!(
            code.contains("-> Result<()>"),
            "handle_empty_response must return Result<()>"
        );
    }

    #[test]
    fn delete_no_response_uses_delete_empty() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: None,
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
            response_type: None, // 204 No Content
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("-> Result<()>"),
            "DELETE with no response should return Result<()>, got:\n{code}"
        );
        assert!(
            code.contains("self.delete_empty("),
            "DELETE with no response should use delete_empty helper, got:\n{code}"
        );
    }

    #[test]
    fn delete_with_response_uses_delete() {
        let op = Operation {
            id: "delete_item".into(),
            method: HttpMethod::Delete,
            path: "/items/{id}".into(),
            summary: None,
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
            response_type: Some(RustType::Named("DeleteResult".into())),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("-> Result<DeleteResult>"),
            "DELETE with response type should return Result<DeleteResult>, got:\n{code}"
        );
        assert!(
            code.contains("self.delete("),
            "DELETE with response type should use regular delete helper, got:\n{code}"
        );
    }

    #[test]
    fn put_no_response_uses_put_no_response() {
        let op = Operation {
            id: "update_item".into(),
            method: HttpMethod::Put,
            path: "/items/{id}".into(),
            summary: None,
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![],
                type_name: Some("UpdateItemRequest".into()),
            }),
            response_type: None, // 204 No Content
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(
            code.contains("-> Result<()>"),
            "PUT with no response should return Result<()>, got:\n{code}"
        );
        assert!(
            code.contains("self.put_no_response("),
            "PUT with no response should use put_no_response helper, got:\n{code}"
        );
    }

    // -- http_method_fn_empty tests --

    #[test]
    fn http_method_fn_empty_get() {
        assert_eq!(http_method_fn_empty(HttpMethod::Get, false), "get_empty");
    }

    #[test]
    fn http_method_fn_empty_post_with_body() {
        assert_eq!(
            http_method_fn_empty(HttpMethod::Post, true),
            "post_no_response"
        );
    }

    #[test]
    fn http_method_fn_empty_post_without_body() {
        assert_eq!(
            http_method_fn_empty(HttpMethod::Post, false),
            "post_empty_no_response"
        );
    }

    #[test]
    fn http_method_fn_empty_put() {
        assert_eq!(
            http_method_fn_empty(HttpMethod::Put, true),
            "put_no_response"
        );
    }

    #[test]
    fn http_method_fn_empty_patch() {
        assert_eq!(
            http_method_fn_empty(HttpMethod::Patch, true),
            "patch_no_response"
        );
    }

    #[test]
    fn http_method_fn_empty_delete() {
        assert_eq!(
            http_method_fn_empty(HttpMethod::Delete, false),
            "delete_empty"
        );
    }

    // -- Combined path + query parameters --

    #[test]
    fn generates_path_and_query_params_combined() {
        let op = Operation {
            id: "get_user_repos".into(),
            method: HttpMethod::Get,
            path: "/users/{userId}/repos".into(),
            summary: None,
            description: None,
            parameters: vec![
                OpParameter {
                    name: "userId".into(),
                    rust_name: "user_id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: None,
                },
                OpParameter {
                    name: "page".into(),
                    rust_name: "page".into(),
                    location: ParamLocation::Query,
                    required: false,
                    rust_type: RustType::Option(Box::new(RustType::I64)),
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Value),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("user_id: &str,"));
        assert!(code.contains("page: Option<i64>,"));
        assert!(code.contains("{user_id}"));
        assert!(code.contains("has_query"));
    }

    // -- PATCH method generation --

    #[test]
    fn generates_patch_method_with_body() {
        let op = Operation {
            id: "update_item".into(),
            method: HttpMethod::Patch,
            path: "/items/{id}".into(),
            summary: Some("Partially update item".into()),
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "name".into(),
                    rust_name: "name".into(),
                    rust_type: RustType::String,
                    required: false,
                    description: None,
                    default_value: None,
                }],
                type_name: Some("PatchItemRequest".into()),
            }),
            response_type: Some(RustType::Named("Item".into())),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("pub async fn update_item("));
        assert!(code.contains("self.patch("));
        assert!(code.contains("req: &PatchItemRequest,"));
    }

    // -- Multiple path parameters --

    #[test]
    fn generates_multiple_path_params() {
        let op = Operation {
            id: "get_comment".into(),
            method: HttpMethod::Get,
            path: "/posts/{postId}/comments/{commentId}".into(),
            summary: None,
            description: None,
            parameters: vec![
                OpParameter {
                    name: "postId".into(),
                    rust_name: "post_id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: None,
                },
                OpParameter {
                    name: "commentId".into(),
                    rust_name: "comment_id".into(),
                    location: ParamLocation::Path,
                    required: true,
                    rust_type: RustType::String,
                    description: None,
                },
            ],
            request_body: None,
            response_type: Some(RustType::Named("Comment".into())),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("post_id: &str,"));
        assert!(code.contains("comment_id: &str,"));
        assert!(code.contains("{post_id}"));
        assert!(code.contains("{comment_id}"));
    }

    // -- POST without body and no response --

    #[test]
    fn post_no_body_no_response() {
        let op = Operation {
            id: "trigger_build".into(),
            method: HttpMethod::Post,
            path: "/build".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("-> Result<()>"));
        assert!(code.contains("post_empty_no_response"));
    }

    // -- PATCH no response uses patch_no_response --

    #[test]
    fn patch_no_response() {
        let op = Operation {
            id: "ack_event".into(),
            method: HttpMethod::Patch,
            path: "/events/{id}/ack".into(),
            summary: None,
            description: None,
            parameters: vec![OpParameter {
                name: "id".into(),
                rust_name: "id".into(),
                location: ParamLocation::Path,
                required: true,
                rust_type: RustType::String,
                description: None,
            }],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![],
                type_name: Some("AckRequest".into()),
            }),
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("-> Result<()>"));
        assert!(code.contains("self.patch_no_response("));
    }

    // -- POST with body but no response --

    #[test]
    fn post_with_body_no_response() {
        let op = Operation {
            id: "send_notification".into(),
            method: HttpMethod::Post,
            path: "/notify".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![FieldDef {
                    name: "message".into(),
                    rust_name: "message".into(),
                    rust_type: RustType::String,
                    required: true,
                    description: None,
                    default_value: None,
                }],
                type_name: Some("NotifyRequest".into()),
            }),
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("self.post_no_response("));
        assert!(code.contains("req: &NotifyRequest,"));
    }

    // -- is_option_type helper --

    #[test]
    fn is_option_type_tests() {
        assert!(is_option_type(&RustType::Option(Box::new(RustType::String))));
        assert!(!is_option_type(&RustType::String));
        assert!(!is_option_type(&RustType::Vec(Box::new(RustType::I64))));
    }

    // -- request_body_type_name with no body --

    #[test]
    fn request_body_type_name_no_body_fallback() {
        let op = Operation {
            id: "do_something".into(),
            method: HttpMethod::Post,
            path: "/something".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        assert_eq!(request_body_type_name(&op), "DoSomethingRequest");
    }

    // -- param_type_string for required Option (unwraps) --

    #[test]
    fn param_type_string_required_option_unwraps() {
        let param = OpParameter {
            name: "tag".into(),
            rust_name: "tag".into(),
            location: ParamLocation::Query,
            required: true,
            rust_type: RustType::Option(Box::new(RustType::String)),
            description: None,
        };
        assert_eq!(param_type_string(&param), "String");
    }

    // -- param_type_string for not-required non-Option wraps in Option --

    #[test]
    fn param_type_string_not_required_non_option_wraps() {
        let param = OpParameter {
            name: "limit".into(),
            rust_name: "limit".into(),
            location: ParamLocation::Query,
            required: false,
            rust_type: RustType::I64,
            description: None,
        };
        assert_eq!(param_type_string(&param), "Option<i64>");
    }

    // -- User agent contains version --

    #[test]
    fn user_agent_includes_version() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(code.contains("pleme-io/test_api 1.0.0"));
    }

    // -- GET with no response body --

    #[test]
    fn get_no_response_uses_get_empty() {
        let op = Operation {
            id: "ping".into(),
            method: HttpMethod::Get,
            path: "/ping".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: None,
            response_type: None,
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("self.get_empty("));
        assert!(code.contains("-> Result<()>"));
    }

    // -- Basic auth in generated helpers --

    #[test]
    fn http_helpers_include_basic_auth() {
        let spec = make_spec("TestApi", AuthMethod::Basic, vec![]);
        let code = generate(&spec);
        assert!(code.contains(".basic_auth(&self.api_key, Option::<&str>::None)"));
    }

    // -- No auth -- no auth call in helpers --

    #[test]
    fn http_helpers_no_auth_has_no_auth_call() {
        let spec = make_spec("TestApi", AuthMethod::None, vec![]);
        let code = generate(&spec);
        assert!(!code.contains("bearer_auth"));
        assert!(!code.contains("basic_auth"));
        assert!(!code.contains(".header(\""));
    }

    // -- Static path with body (no path/query params) --

    #[test]
    fn static_path_with_body() {
        let op = Operation {
            id: "create_widget".into(),
            method: HttpMethod::Post,
            path: "/widgets".into(),
            summary: None,
            description: None,
            parameters: vec![],
            request_body: Some(OpRequestBody {
                required: true,
                fields: vec![],
                type_name: Some("Widget".into()),
            }),
            response_type: Some(RustType::Named("Widget".into())),
            errors: vec![],
        };
        let spec = make_spec("TestApi", AuthMethod::None, vec![op]);
        let code = generate(&spec);
        assert!(code.contains("self.post(\"/widgets\", req).await"));
    }

    // -- The path template is parsed, not string-replaced --

    #[test]
    fn path_template_captures_declared_parameters() {
        let param = OpParameter {
            name: "itemId".into(),
            rust_name: "item_id".into(),
            location: ParamLocation::Path,
            required: true,
            rust_type: RustType::String,
            description: None,
        };
        let params = vec![&param];
        let t = path_template("/items/{itemId}/tags", &params);
        assert_eq!(t.text(), "/items/{item_id}/tags");
    }
}
