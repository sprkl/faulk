//! The helper's MCP server (AG-1): one tool per command, generated from the command catalog
//! (`commands::catalog`), each call relayed to the page that owns the project.
//!
//! Streamable HTTP on the helper's own port, path [`PATH`]. Each request is checked before the
//! MCP server sees it ([`crate::security::Policy::check_mcp`]): its `Host` names the helper, it
//! carries no `Origin` (no web page drives the MCP server) and it holds the MCP token
//! (`Authorization: Bearer`, `<config dir>/mcp-token`). The MCP library checks `Host` and
//! `Origin` again. Stdio is `faulk-helper mcp`, a bridge to this server ([`crate::mcp_stdio`]).
//!
//! The helper runs no command (the page owns the project in both modes): a tool call becomes an
//! `agent.call` to the page (its Boost session, else its control link: #608, in Web mode as in
//! Boost, with or without an audio output), which the page runs through its dispatcher like a
//! user's action, as `agent:<client name>`, and answers with `agent.result`
//! ([`crate::server::Shared::agent_call`]). No paired page open → the tool answers the error
//! `page.absent` at once.

use crate::security::McpRefusal;
use crate::server::{RelayError, Shared};
use bytes::Bytes;
use http::{Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::body::Incoming;
use protocol::AgentResult;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
    Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, OnceLock, Weak};
use tokio::net::TcpStream;
use tower_service::Service;

/// The MCP server's path on the helper's port.
pub const PATH: &str = "/mcp";

/// The name of a command's tool: the command's name with `.` replaced by `_`
/// (`track.create` → `track_create`). Model APIs and several MCP clients refuse dots in a
/// tool's name; the mapping is one to one over the catalog (tested).
pub fn tool_name(command: &str) -> String {
    command.replace('.', "_")
}

/// The tools, built once from the catalog, and the command each tool's name stands for.
struct Tools {
    list: Vec<Tool>,
    commands: HashMap<String, String>,
}

fn tools() -> &'static Tools {
    static TOOLS: OnceLock<Tools> = OnceLock::new();
    TOOLS.get_or_init(|| {
        let mut list = Vec::new();
        let mut commands = HashMap::new();
        for spec in commands::catalog() {
            let name = tool_name(&spec.name);
            let mut tool = Tool::new_with_raw(
                name.clone(),
                Some(spec.description.clone().into()),
                object(spec.input_schema),
            );
            if let Value::Object(output) = spec.output_schema {
                tool.output_schema = Some(Arc::new(output));
            }
            tool.annotations = Some(
                ToolAnnotations::new()
                    .read_only(spec.kind == commands::Kind::Query)
                    .destructive(spec.destructive),
            );
            commands.insert(name, spec.name);
            list.push(tool);
        }
        Tools { list, commands }
    })
}

fn object(value: Value) -> Arc<Map<String, Value>> {
    match value {
        Value::Object(map) => Arc::new(map),
        _ => Arc::new(Map::new()),
    }
}

/// The MCP tools the server lists: one per command, in the catalog's order.
pub fn tool_list() -> &'static [Tool] {
    &tools().list
}

/// The command a tool's name stands for; `None` for a name that is no tool.
pub fn command_of(tool: &str) -> Option<&'static str> {
    tools().commands.get(tool).map(String::as_str)
}

/// The history's author name of an MCP client: its `clientInfo.name`, kept to letters, digits,
/// space, `.`, `_` and `-` (at most 64 characters); `mcp` when nothing is left.
pub fn agent_name(client: Option<&str>) -> String {
    let name: String = client
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(64)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        "mcp".to_owned()
    } else {
        name.to_owned()
    }
}

/// The command JSON a tool call stands for: the arguments with the command's `type`. `Err` is
/// the message of an invalid call (the arguments name a `type` of their own: the tool's name
/// says which command it is).
pub fn command_json(command: &str, arguments: Option<Map<String, Value>>) -> Result<Value, String> {
    let mut arguments = arguments.unwrap_or_default();
    if arguments.contains_key("type") {
        return Err("the arguments have no `type`: the tool's name names the command".to_owned());
    }
    arguments.insert("type".to_owned(), Value::String(command.to_owned()));
    Ok(Value::Object(arguments))
}

/// What an agent's tool call answers: the command's output on success; otherwise an error
/// result whose text is a JSON object with a stable `code` (`page.absent`, `page.gone`,
/// `page.timeout`, `page.unavailable`, `command.refused`, `command.invalid`), a `message` and,
/// for a refusal, the command's `error`.
pub fn tool_result(result: Result<AgentResult, RelayError>) -> CallToolResult {
    let error = |code: &str, message: String, detail: Option<Value>| {
        let mut body = json!({ "code": code, "message": message });
        if let Some(detail) = detail {
            body["error"] = detail;
        }
        CallToolResult::error(vec![ContentBlock::text(body.to_string())])
    };
    match result {
        Ok(AgentResult::Applied { output }) => match output {
            Value::Object(mut fields) => {
                // The output's `type` is the tool's own name: the output schema has none.
                fields.remove("type");
                let output = Value::Object(fields);
                let mut answer =
                    CallToolResult::success(vec![ContentBlock::text(output.to_string())]);
                answer.structured_content = Some(output);
                answer
            }
            _ => CallToolResult::success(vec![ContentBlock::text("done")]),
        },
        Ok(AgentResult::Refused { error: detail }) => error(
            "command.refused",
            "the project refused the command: nothing changed".to_owned(),
            Some(detail),
        ),
        Ok(AgentResult::Invalid { message }) => error("command.invalid", message, None),
        Ok(AgentResult::Unavailable { reason }) => error("page.unavailable", reason, None),
        Err(relay) => error(relay.code(), relay.message().to_owned(), None),
    }
}

const INSTRUCTIONS: &str = "Faulk's commands, one tool per command. Each call runs in the Faulk \
page open in the browser and paired with this helper (whether the browser or the helper plays \
the sound), exactly as if the user did it: it appears in the page's history and can be undone \
there. Without a paired page open every tool answers the error `page.absent`: ask the user to \
open Faulk in the browser.";

/// The MCP handler of one MCP session: lists the tools, relays their calls to the page.
#[derive(Clone)]
pub struct Handler {
    shared: Weak<Shared>,
}

impl Handler {
    pub fn new(shared: &Arc<Shared>) -> Self {
        Self {
            shared: Arc::downgrade(shared),
        }
    }

    /// Calls the tool `name` with `arguments` for the agent `agent`: the call's tool result, or
    /// `Err` with the message of a call that names no tool or has invalid arguments.
    pub async fn call(
        &self,
        agent: &str,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let command = command_of(name).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("no tool named {name}"), None)
        })?;
        let command = command_json(command, arguments)
            .map_err(|message| rmcp::ErrorData::invalid_params(message, None))?;
        let Some(shared) = self.shared.upgrade() else {
            return Ok(tool_result(Err(RelayError::Absent)));
        };
        Ok(tool_result(shared.agent_call(agent, command).await))
    }
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("faulk", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult::with_all_items(tool_list().to_vec()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let client = context.peer.peer_info();
        let agent = agent_name(client.as_ref().map(|info| info.client_info.name.as_str()));
        self.call(&agent, &request.name, request.arguments)
            .await
            .map(CallToolResponse::from)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tool_list().iter().find(|t| t.name == name).cloned()
    }
}

type Mcp = StreamableHttpService<Handler, LocalSessionManager>;

/// The MCP server of a helper: one for every connection, so that an MCP session outlives the
/// HTTP connection it started on.
pub struct McpHttp(Mcp);

impl McpHttp {
    pub fn new(shared: &Arc<Shared>) -> Self {
        let port = shared.policy.port;
        let handler = Handler::new(shared);
        let config = StreamableHttpServerConfig::default()
            .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
            .enforce_origin_validation();
        Self(StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::new(LocalSessionManager::default()),
            config,
        ))
    }
}

type Body = BoxBody<Bytes, Infallible>;

fn refusal(status: StatusCode) -> Response<Body> {
    let mut response = Response::new(Full::new(Bytes::new()).boxed());
    *response.status_mut() = status;
    response.headers_mut().insert(
        crate::server::HELPER_HEADER,
        header::HeaderValue::from_static(protocol::HELPER_NAME),
    );
    response
}

/// Serves one HTTP connection to the MCP server: each request is checked, then given to it.
pub async fn serve_connection(stream: TcpStream, shared: Arc<Shared>, mcp: &McpHttp) {
    let mcp = mcp.0.clone();
    let service = hyper::service::service_fn(move |request: Request<Incoming>| {
        let header =
            |name: header::HeaderName| request.headers().get(name).and_then(|v| v.to_str().ok());
        let check = shared.policy.check_mcp(&crate::security::McpRequest {
            host: header(header::HOST),
            has_origin: request.headers().contains_key(header::ORIGIN),
            authorization: header(header::AUTHORIZATION),
        });
        // A kept-alive connection may ask for another path next: only `/mcp` reaches the server.
        let on_path = request.uri().path() == PATH;
        let mut mcp = mcp.clone();
        async move {
            match check {
                Ok(()) if on_path => mcp.call(request).await,
                Ok(()) => Ok(refusal(StatusCode::NOT_FOUND)),
                Err(why) => {
                    crate::log!("refused an MCP request: {why:?}");
                    let status = match why {
                        McpRefusal::BadToken | McpRefusal::NoToken => StatusCode::UNAUTHORIZED,
                        McpRefusal::BadHost | McpRefusal::Origin => StatusCode::FORBIDDEN,
                    };
                    Ok::<_, Infallible>(refusal(status))
                }
            }
        }
    });
    let io = hyper_util::rt::TokioIo::new(stream);
    if let Err(e) = hyper::server::conn::http1::Builder::new()
        .serve_connection(io, service)
        .await
    {
        crate::log!("MCP connection ended: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn one_tool_per_command_and_each_name_leads_back_to_its_command() {
        let catalog = commands::catalog();
        assert_eq!(tool_list().len(), catalog.len());
        let mut names = HashSet::new();
        for (tool, spec) in tool_list().iter().zip(&catalog) {
            assert!(names.insert(tool.name.to_string()), "{} twice", tool.name);
            assert_eq!(command_of(&tool.name), Some(spec.name.as_str()));
            assert!(
                tool.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "{}",
                tool.name
            );
            assert!(tool.name.len() <= 64, "{}", tool.name);
            assert_eq!(tool.description.as_deref(), Some(spec.description.as_str()));
            assert_eq!(
                Value::Object((*tool.input_schema).clone()),
                spec.input_schema
            );
            let annotations = tool.annotations.as_ref().unwrap();
            assert_eq!(
                annotations.destructive_hint,
                Some(spec.destructive),
                "{}",
                spec.name
            );
            assert_eq!(
                annotations.read_only_hint,
                Some(spec.kind == commands::Kind::Query),
                "{}",
                spec.name
            );
        }
        assert_eq!(
            command_of("track.create"),
            None,
            "the command's own name is no tool"
        );
        assert_eq!(command_of("nope"), None);
    }

    #[test]
    fn an_agents_name_is_kept_to_a_safe_alphabet() {
        assert_eq!(agent_name(Some("claude-code")), "claude-code");
        assert_eq!(agent_name(Some("Claude Code 2.1")), "Claude Code 2.1");
        assert_eq!(agent_name(Some("a:b\n<script>")), "abscript");
        assert_eq!(agent_name(Some("  ")), "mcp");
        assert_eq!(agent_name(Some(":::")), "mcp");
        assert_eq!(agent_name(None), "mcp");
        assert_eq!(agent_name(Some(&"x".repeat(200))).len(), 64);
    }

    #[test]
    fn a_calls_arguments_become_the_command_with_its_type() {
        let mut args = Map::new();
        args.insert("name".into(), json!("Lead"));
        assert_eq!(
            command_json("track.rename", Some(args)).unwrap(),
            json!({ "type": "track.rename", "name": "Lead" })
        );
        assert_eq!(
            command_json("history.undo", None).unwrap(),
            json!({ "type": "history.undo" })
        );
        let mut sneaky = Map::new();
        sneaky.insert("type".into(), json!("project.new"));
        assert!(command_json("history.undo", Some(sneaky)).is_err());
    }

    fn text(result: &CallToolResult) -> Value {
        let ContentBlock::Text(t) = &result.content[0] else {
            panic!("text content")
        };
        serde_json::from_str(&t.text).unwrap_or(Value::String(t.text.clone()))
    }

    #[test]
    fn an_applied_command_answers_its_output_without_its_type() {
        let ok = tool_result(Ok(AgentResult::Applied {
            output: json!({ "type": "track.create", "track": "t1" }),
        }));
        assert_eq!(ok.is_error, Some(false));
        assert_eq!(ok.structured_content, Some(json!({ "track": "t1" })));
        assert_eq!(text(&ok), json!({ "track": "t1" }));
        let later = tool_result(Ok(AgentResult::Applied {
            output: Value::Null,
        }));
        assert_eq!(later.is_error, Some(false));
        assert_eq!(later.structured_content, None);
    }

    #[test]
    fn every_failure_answers_an_error_with_a_stable_code() {
        let refused = tool_result(Ok(AgentResult::Refused {
            error: json!({ "code": "not_found", "entity": "track", "id": "x" }),
        }));
        assert_eq!(refused.is_error, Some(true));
        assert_eq!(refused.structured_content, None);
        assert_eq!(text(&refused)["code"], "command.refused");
        assert_eq!(text(&refused)["error"]["code"], "not_found");
        for (result, code) in [
            (
                Ok(AgentResult::Invalid {
                    message: "unknown variant".into(),
                }),
                "command.invalid",
            ),
            (
                Ok(AgentResult::Unavailable {
                    reason: "no project yet".into(),
                }),
                "page.unavailable",
            ),
            (Err(RelayError::Absent), "page.absent"),
            (Err(RelayError::Gone), "page.gone"),
            (Err(RelayError::Timeout), "page.timeout"),
        ] {
            let answer = tool_result(result);
            assert_eq!(answer.is_error, Some(true), "{code}");
            assert_eq!(text(&answer)["code"], code);
            assert!(
                text(&answer)["message"].as_str().unwrap().len() > 5,
                "{code}"
            );
        }
    }
}
