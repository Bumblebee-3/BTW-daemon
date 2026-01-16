use serde::Deserialize;

/// Minimal view of a command from the authoritative commands.json
#[derive(Debug, Deserialize)]
pub struct Command { pub id: String }

/// Parse commands.json for counting/logging purposes
pub fn parse_commands_json(s: &str) -> Result<Vec<Command>, String> {
    serde_json::from_str::<Vec<Command>>(s).map_err(|e| e.to_string())
}
