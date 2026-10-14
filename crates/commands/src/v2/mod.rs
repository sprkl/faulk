//! Command contract v2 (CT-4). PR-2 promotes this module to the crate root and
//! migrates the v1 commands of `lib.rs` into their domains.
//!
//! One file per domain (spec § 5.2): a task adds its commands to its domain's file and
//! never touches the root, whose [`Command`] counts one line per domain. The root
//! derives names, metadata, schema and the MCP tool [`catalog`] from the domains.

pub mod contract;
pub(crate) mod macros;
pub mod testing;

pub mod arrangement;
pub mod automation;
pub mod clip;
pub mod device;
pub mod history;
pub mod launch;
pub mod live;
pub mod midi;
pub mod mixer;
pub mod note;
pub mod project;
pub mod record;
pub mod scene;
pub mod track;
pub mod transport;
pub mod view;

pub use contract::{
    Author, CommandError, Context, ControlMsg, GroupId, Kind, Meta, ProjectEvent, State, Undo,
    UndoKind,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

macros::root! {
    Project => project,
    Track => track,
    Device => device,
    Clip => clip,
    Note => note,
    Scene => scene,
    Launch => launch,
    Arrangement => arrangement,
    Automation => automation,
    Transport => transport,
    Mixer => mixer,
    Record => record,
    Midi => midi,
    Live => live,
    History => history,
    View => view,
}

/// What applying a root [`Command`] produces.
pub type Outcome = contract::Outcome<Command, Output>;

/// The JSON Schema of every command's input, exported to the web app and to agents.
pub fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Command)).unwrap_or_default()
}

/// The JSON Schema of every command's output.
pub fn output_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Output)).unwrap_or_default()
}

/// One MCP tool: what the MCP server (AG-1) exposes for a command, derived from the
/// schema and never written by hand.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct ToolSpec {
    /// The command's name, `domain.verb`, also the tool's name.
    pub name: String,
    /// The command's description, for agents.
    pub description: String,
    pub kind: Kind,
    pub undo: UndoKind,
    pub destructive: bool,
    /// The command's fields without `type` (the tool's name carries it); `$defs`
    /// holds the referenced types.
    pub input_schema: Value,
    /// The output's fields without `type`; `$defs` holds the referenced types.
    pub output_schema: Value,
}

/// The MCP tools, one per command, in domain then declaration order.
pub fn catalog() -> Vec<ToolSpec> {
    let input = schema();
    let output = output_schema();
    let outputs: Vec<(String, Value)> = variants(&output)
        .into_iter()
        .map(|v| (variant_name(&v).to_owned(), tool_schema(&v, &output)))
        .collect();
    variants(&input)
        .into_iter()
        .map(|v| {
            let name = variant_name(&v).to_owned();
            let output_schema = outputs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| s.clone())
                .unwrap_or(Value::Null);
            ToolSpec {
                description: v["description"].as_str().unwrap_or_default().to_owned(),
                kind: serde_json::from_value(v["x-kind"].clone()).unwrap_or(Kind::Edit),
                undo: serde_json::from_value(v["x-undo"].clone()).unwrap_or(UndoKind::Inverse),
                destructive: v["x-destructive"].as_bool().unwrap_or(true),
                input_schema: tool_schema(&v, &input),
                output_schema,
                name,
            }
        })
        .collect()
}

/// The variant schemas of a root (untagged) schema: each domain's `oneOf`, in order.
fn variants(root: &Value) -> Vec<Value> {
    let defs = &root["$defs"];
    root["anyOf"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|domain| {
            let def = domain["$ref"]
                .as_str()
                .and_then(|r| r.strip_prefix("#/$defs/"))
                .map(|name| &defs[name])
                .unwrap_or(domain);
            def["oneOf"].as_array().cloned().unwrap_or_default()
        })
        .collect()
}

fn variant_name(variant: &Value) -> &str {
    variant["properties"]["type"]["const"]
        .as_str()
        .unwrap_or_default()
}

/// A variant schema as a standalone tool schema: no `type` field, the root `$defs`.
fn tool_schema(variant: &Value, root: &Value) -> Value {
    let mut properties = variant["properties"].clone();
    if let Some(map) = properties.as_object_mut() {
        map.remove("type");
    }
    let required: Vec<&Value> = variant["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r.as_str() != Some("type"))
        .collect();
    let mut out = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    });
    if let Some(defs) = root.get("$defs") {
        out["$defs"] = defs.clone();
    }
    out
}
