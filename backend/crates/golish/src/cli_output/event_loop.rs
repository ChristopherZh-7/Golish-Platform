use std::io::{self, Write};

use anyhow::Result;
use tokio::sync::mpsc;

use golish_core::events::AiEvent;
use golish_core::runtime::RuntimeEvent;

use super::cli_json::convert_to_cli_json;
use super::terminal::handle_ai_event_terminal;

/// Run the event loop, processing events until completion or error.
///
/// This function consumes the event receiver and processes events until
/// it sees a `Completed` or `Error` event from the AI agent.
///
/// # Arguments
///
/// * `event_rx` - Channel receiver for runtime events
/// * `json_mode` - If true, output events as JSON lines
/// * `quiet_mode` - If true, only output final response
pub async fn run_event_loop(
    mut event_rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    json_mode: bool,
    quiet_mode: bool,
) -> Result<()> {
    while let Some(event) = event_rx.recv().await {
        match event {
            RuntimeEvent::Ai {
                event: ai_event, ..
            } => {
                let should_break = handle_ai_event(&ai_event, json_mode, quiet_mode)?;
                if should_break {
                    break;
                }
            }
            RuntimeEvent::TerminalOutput { data, .. } => {
                // Write terminal output directly to stdout
                if !quiet_mode && !json_mode {
                    io::stdout().write_all(&data)?;
                    io::stdout().flush()?;
                }
            }
            RuntimeEvent::TerminalExit { session_id, code } => {
                if json_mode {
                    let json = serde_json::json!({
                        "type": "terminal_exit",
                        "session_id": session_id,
                        "code": code
                    });
                    println!("{}", json);
                }
            }
            RuntimeEvent::Custom { name, payload } => {
                if json_mode {
                    let json = serde_json::json!({
                        "type": "custom",
                        "name": name,
                        "payload": payload
                    });
                    println!("{}", json);
                }
            }
            RuntimeEvent::AiEnvelope { envelope, .. } => {
                // Handle enveloped AI events the same as regular AI events
                // The envelope provides seq/ts for reliability but the event
                // content is processed the same way
                let should_break = handle_ai_event(&envelope.event, json_mode, quiet_mode)?;
                if should_break {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Handle an AI event, returning true if the loop should exit.
fn handle_ai_event(event: &AiEvent, json_mode: bool, quiet_mode: bool) -> Result<bool> {
    if json_mode {
        // JSON mode: output standardized CLI JSON format (NO TRUNCATION)
        let cli_json = convert_to_cli_json(event);
        println!("{}", serde_json::to_string(&cli_json)?);
        io::stdout().flush()?;
    } else if !quiet_mode {
        // Terminal mode: pretty-print events with box-drawing format
        handle_ai_event_terminal(event)?;
    }

    // Check for completion/error events
    match event {
        AiEvent::Completed { response, .. } => {
            if quiet_mode && !json_mode {
                // In quiet mode, only print the final response
                println!("{}", response);
            } else if !json_mode {
                // Ensure we end with a newline after streaming
                println!();
            }
            Ok(true) // Exit loop
        }
        AiEvent::Error { message, .. } => {
            if !json_mode {
                eprintln!("Error: {}", message);
            }
            Ok(true) // Exit loop
        }
        _ => Ok(false), // Continue loop
    }
}
