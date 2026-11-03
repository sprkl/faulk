//! Every Faulk user action as a typed command (contract v2, CT-4; promoted here by PR-2).
//!
//! The UI, the MCP tools and the ACP agents all act through [`Command`]: the UI has no
//! private path to the engine or the project. A command applies to the [`State`]
//! (project, session, history) and returns an [`Outcome`]: its typed output, its
//! inverse, its events for the interface and its messages for the engine. Hosts run
//! commands through [`run`], which keeps the undo history ([`history`]).
//!
//! One file per domain (spec § 5.2): a task adds its commands to its domain's file and
//! never touches the root, whose [`Command`] counts one line per domain. The root
//! derives names, metadata, schema and the MCP tool [`catalog`] from the domains.

pub mod contract;
pub mod describe;
pub mod engine_msgs;
pub mod export;
pub mod host;
pub(crate) mod macros;
pub mod testing;

pub mod arrangement;
pub mod automation;
pub mod clip;
pub mod device;
pub mod history;
pub mod launch;
pub mod live;
pub mod marker;
pub mod midi;
pub mod mixer;
pub mod note;
pub mod preset;
pub mod project;
pub mod query;
pub mod record;
pub mod scene;
pub mod track;
pub mod transport;
pub mod view;

pub use contract::{
    Author, CommandError, Context, ControlMsg, GroupId, Kind, Meta, ProjectEvent, State, TakeState,
    Undo, UndoKind,
};
pub use history::{History, run};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};

macros::root! {
    Project => project,
    Track => track,
    Device => device,
    Preset => preset,
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
    /// holds the types they reference, directly or not, and only those (absent if none).
    pub input_schema: Value,
    /// The output's fields without `type`; `$defs` as in `input_schema`.
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
            // The `domain!` macro writes the three extensions on every command: a
            // missing one is a programming error, never a default.
            ToolSpec {
                description: v["description"].as_str().unwrap_or_default().to_owned(),
                kind: serde_json::from_value(v["x-kind"].clone())
                    .unwrap_or_else(|e| panic!("{name}: x-kind: {e}")),
                undo: serde_json::from_value(v["x-undo"].clone())
                    .unwrap_or_else(|e| panic!("{name}: x-undo: {e}")),
                destructive: v["x-destructive"]
                    .as_bool()
                    .unwrap_or_else(|| panic!("{name}: x-destructive missing")),
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

/// A variant schema as a standalone tool schema: no `type` field, and the definitions of the
/// root `$defs` that its fields reach, directly or through other definitions. Copying the
/// whole root `$defs` into every tool made the catalog grow with the square of the number
/// of commands (#559): 55 MB for 136 tools.
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
    if let Some(defs) = root.get("$defs").and_then(Value::as_object) {
        let reached = reachable_defs(&out, defs);
        if !reached.is_empty() {
            out["$defs"] = Value::Object(reached);
        }
    }
    out
}

/// The definitions of `defs` that `schema` references through `#/$defs/<name>`, and those
/// they reference in turn, each copied unchanged. A reference to a name `defs` lacks is left
/// dangling, as it was in the root.
fn reachable_defs(
    schema: &Value,
    defs: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let mut reached = std::collections::HashSet::new();
    let mut pending = Vec::new();
    def_refs(schema, &mut pending);
    while let Some(name) = pending.pop() {
        if let Some(def) = defs.get(&name)
            && reached.insert(name)
        {
            def_refs(def, &mut pending);
        }
    }
    // Same order as the root, so that the output does not depend on the walk.
    defs.iter()
        .filter(|(name, _)| reached.contains(*name))
        .map(|(name, def)| (name.clone(), def.clone()))
        .collect()
}

/// Pushes the name of every `#/$defs/<name>` reference found in `value`, at any depth.
fn def_refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                match (key.as_str(), v) {
                    ("$ref", Value::String(r)) => {
                        // `#/$defs/A/properties/b` needs the whole of `A`.
                        if let Some(path) = r.strip_prefix("#/$defs/") {
                            let name = path.split('/').next().unwrap_or(path);
                            out.push(name.replace("~1", "/").replace("~0", "~"));
                        }
                    }
                    _ => def_refs(v, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| def_refs(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> serde_json::Map<String, Value> {
        json!({
            "A": { "type": "object", "properties": { "b": { "$ref": "#/$defs/B" } } },
            "B": { "anyOf": [{ "$ref": "#/$defs/A" }, { "type": "null" }] },
            "C": { "items": [{ "$ref": "#/$defs/D" }] },
            "D": { "type": "string" },
            "E/F": { "type": "integer" },
            "Unused": { "type": "boolean" }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn reached(schema: Value) -> Vec<String> {
        reachable_defs(&schema, &defs()).keys().cloned().collect()
    }

    #[test]
    fn keeps_what_the_fields_reach_through_cycles_arrays_and_deep_pointers() {
        assert_eq!(reached(json!({ "$ref": "#/$defs/B" })), ["A", "B"]);
        assert_eq!(
            reached(json!({ "items": [{ "$ref": "#/$defs/C" }] })),
            ["C", "D"]
        );
        // A pointer into a definition needs the whole definition.
        assert_eq!(
            reached(json!({ "$ref": "#/$defs/A/properties/b" })),
            ["A", "B"]
        );
        assert_eq!(reached(json!({ "$ref": "#/$defs/E~1F" })), ["E/F"]);
        // `$ref` as a property name is a field, not a reference.
        assert_eq!(
            reached(json!({ "properties": { "$ref": { "$ref": "#/$defs/D" } } })),
            ["D"]
        );
    }

    #[test]
    fn keeps_nothing_for_no_reference_or_an_unknown_one() {
        assert!(reached(json!({ "type": "object" })).is_empty());
        assert!(reached(json!({ "$ref": "#/$defs/Missing" })).is_empty());
        assert!(reached(json!({ "$ref": "https://example.com/A" })).is_empty());
    }

    #[test]
    fn a_tool_schema_omits_an_empty_defs_and_keeps_the_root_order() {
        let root = json!({ "$defs": defs() });
        let plain = json!({ "properties": { "type": { "const": "x.y" }, "n": { "type": "integer" } }, "required": ["type", "n"] });
        let tool = tool_schema(&plain, &root);
        assert!(tool.get("$defs").is_none(), "{tool}");
        assert_eq!(tool["required"], json!(["n"]));
        let with_refs =
            json!({ "properties": { "d": { "$ref": "#/$defs/D" }, "b": { "$ref": "#/$defs/B" } } });
        let keys: Vec<_> = tool_schema(&with_refs, &root)["$defs"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, ["A", "B", "D"]);
    }
}
