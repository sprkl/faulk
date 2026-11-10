//! `faulk mcp <folder> --stdio`: the command catalog as MCP tools, over stdio, each call run by
//! the headless host on its folder (no page, no relay).
//!
//! The tools are generated from `commands::catalog()` as the helper's (`crates/helper/src/mcp.rs`:
//! one tool per command, `.` replaced by `_`, the command's schemas and annotations), minus the
//! commands the headless host does not run ([`crate::policy`]). A call runs as
//! `agent:<clientInfo.name>`; after each call that changed the project the host saves the folder,
//! and the session's end saves it again and ends the agent's turn.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use serde_json::{Map, Value, json};

use crate::host::{Answer, Host};
use crate::policy::{self, Route};

/// The name of a command's tool (the helper's `tool_name`).
pub fn tool_name(command: &str) -> String {
    command.replace('.', "_")
}

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
            if matches!(policy::route(&spec.name, spec.kind), Route::Refused(_)) {
                continue;
            }
            let name = tool_name(&spec.name);
            let schema = match spec.input_schema {
                Value::Object(map) => Arc::new(map),
                _ => Arc::new(Map::new()),
            };
            let mut tool =
                Tool::new_with_raw(name.clone(), Some(spec.description.clone().into()), schema);
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

/// The tools the headless server lists.
pub fn tool_list() -> &'static [Tool] {
    &tools().list
}

/// The command a tool's name stands for; `None` for a name that is no tool of this server.
pub fn command_of(tool: &str) -> Option<&'static str> {
    tools().commands.get(tool).map(String::as_str)
}

/// The history's author of an MCP client (the helper's `agent_name`, prefixed): its
/// `clientInfo.name` kept to letters, digits, space, `.`, `_` and `-`, `mcp` when nothing is left.
pub fn agent_author(client: Option<&str>) -> String {
    let name: String = client
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(64)
        .collect();
    let name = name.trim();
    format!("agent:{}", if name.is_empty() { "mcp" } else { name })
}

/// The command JSON a tool call stands for (the helper's `command_json`).
pub fn command_json(command: &str, arguments: Option<Map<String, Value>>) -> Result<Value, String> {
    let mut arguments = arguments.unwrap_or_default();
    if arguments.contains_key("type") {
        return Err("the arguments have no `type`: the tool's name names the command".to_owned());
    }
    arguments.insert("type".to_owned(), Value::String(command.to_owned()));
    Ok(Value::Object(arguments))
}

/// What a tool call answers: the command's output on success; otherwise an error whose text is a
/// JSON object with a stable `code` (`command.refused`, `command.invalid`,
/// [`policy::UNSUPPORTED`], `host.failed`), a `message` and, for a refusal, the command's `error`.
pub fn tool_result(answer: Answer) -> CallToolResult {
    let error = |code: &str, message: String, detail: Option<Value>| {
        let mut body = json!({ "code": code, "message": message });
        if let Some(detail) = detail {
            body["error"] = detail;
        }
        CallToolResult::error(vec![ContentBlock::text(body.to_string())])
    };
    match answer {
        Answer::Applied { output, file } => match output {
            Value::Object(mut fields) => {
                fields.remove("type");
                let output = Value::Object(fields);
                let mut content = vec![ContentBlock::text(output.to_string())];
                if let Some(file) = file {
                    content.push(ContentBlock::text(format!(
                        "written: {file} (in the project's folder)"
                    )));
                }
                let mut result = CallToolResult::success(content);
                result.structured_content = Some(output);
                result
            }
            _ => CallToolResult::success(vec![ContentBlock::text("done")]),
        },
        Answer::Refused { error: detail } => error(
            "command.refused",
            "the command was refused: nothing changed".to_owned(),
            Some(detail),
        ),
        Answer::Invalid { message } => error("command.invalid", message, None),
        Answer::Unsupported { message } => error(policy::UNSUPPORTED, message, None),
        Answer::Failed { message } => error("host.failed", message, None),
    }
}

const INSTRUCTIONS: &str = "Faulk's commands, one tool per command, run on one project folder \
by the headless Faulk host (no browser): what you do is saved into the folder after each call, \
and the folder opens in Faulk's page afterwards. Your consecutive edits are one entry of the \
project's history (one undo undoes them), until 30 s pass without an edit of yours. \
`project_export_audio` writes a WAV file into the folder's `exports/` and names it in its \
answer; `project_analyze_render` renders offline and measures the levels. Commands that need \
a page, a live engine, an audio or MIDI device or the browser's storage are not offered; \
`project_new` and `project_open` neither: the host works on the folder it was started on.";

/// The MCP handler: the tools, run by the shared host.
#[derive(Clone)]
pub struct Handler {
    host: Arc<Mutex<Host>>,
    /// The authors of this session's calls, whose turn ends with the session.
    authors: Arc<Mutex<Vec<String>>>,
}

impl Handler {
    pub fn new(host: Arc<Mutex<Host>>) -> Self {
        Self {
            host,
            authors: Arc::default(),
        }
    }

    /// Runs the tool `name` as `author`: the tool's result, or `Err` for a name that is no tool
    /// or arguments that name a `type`.
    pub fn call(
        &self,
        author: &str,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let command = command_of(name).ok_or_else(|| {
            rmcp::ErrorData::invalid_params(format!("no tool named {name}"), None)
        })?;
        let command = command_json(command, arguments)
            .map_err(|message| rmcp::ErrorData::invalid_params(message, None))?;
        let mut host = self.host.lock().unwrap_or_else(|p| p.into_inner());
        let mut answer = host.call(author, &command);
        if answer.applied() && host.dirty() {
            // What an agent did is in the folder before it is answered: a crash loses nothing.
            if let Err(e) = host.save() {
                answer = Answer::Failed {
                    message: format!("the command applied but the folder was not saved: {e}"),
                };
            }
        }
        drop(host);
        let mut authors = self.authors.lock().unwrap_or_else(|p| p.into_inner());
        if !authors.iter().any(|a| a == author) {
            authors.push(author.to_owned());
        }
        Ok(tool_result(answer))
    }

    /// The session ended: the turns of its agents end and the folder is saved.
    pub fn end(&self) -> Result<(), String> {
        let mut host = self.host.lock().unwrap_or_else(|p| p.into_inner());
        for author in self
            .authors
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
        {
            host.agent_left(&author);
        }
        host.save().map(|_| ()).map_err(|e| e.to_string())
    }
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        rmcp::model::InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
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
        let author = agent_author(client.as_ref().map(|info| info.client_info.name.as_str()));
        let handler = self.clone();
        // A render runs for seconds: off the runtime's thread.
        tokio::task::spawn_blocking(move || handler.call(&author, &request.name, request.arguments))
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?
            .map(CallToolResponse::from)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tool_list().iter().find(|t| t.name == name).cloned()
    }
}

/// Serves the host over stdio until the client closes it, then saves the folder.
pub async fn serve_stdio(host: Host) -> Result<(), String> {
    let handler = Handler::new(Arc::new(Mutex::new(host)));
    let running = handler
        .clone()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| format!("the MCP session did not start: {e}"))?;
    let _ = running.waiting().await;
    handler.end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_are_the_catalog_minus_the_refused_commands() {
        let catalog = commands::catalog();
        let offered: Vec<&str> = catalog
            .iter()
            .filter(|s| !matches!(policy::route(&s.name, s.kind), Route::Refused(_)))
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(tool_list().len(), offered.len());
        for name in offered {
            assert_eq!(command_of(&tool_name(name)), Some(name));
        }
        for refused in [
            "project_new",
            "project_open",
            "transport_play",
            "mixer_levels",
        ] {
            assert_eq!(command_of(refused), None, "{refused}");
        }
    }

    #[test]
    fn an_author_is_an_agent_named_after_the_client() {
        assert_eq!(agent_author(Some("claude-code")), "agent:claude-code");
        assert_eq!(agent_author(Some("a\u{0}b/c")), "agent:abc");
        assert_eq!(agent_author(Some("  ")), "agent:mcp");
        assert_eq!(agent_author(None), "agent:mcp");
    }
}
