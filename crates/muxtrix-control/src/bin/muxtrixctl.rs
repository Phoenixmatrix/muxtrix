use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr as _;

use muxtrix_control::{
    Agent, AgentState, ClaudeHook, ControlRequest, Endpoint, HookAction, HookManager, HookScope,
    SplitDirection, send_request,
};
use serde_json::Value;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "hook-event")
    {
        run_hook_event(&arguments[1..]);
        println!("{{}}");
        return ExitCode::SUCCESS;
    }
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("muxtrixctl: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<(), String> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage());
    };
    if matches!(command, "--version" | "-V") {
        println!("muxtrixctl {}", muxtrix_control::VERSION);
        return Ok(());
    }
    if command == "hooks" {
        return run_hooks(&arguments[1..]);
    }
    validate_option_values(arguments, &["--title", "--body", "--pane"])?;

    let request = match command {
        "ping" => ControlRequest::Ping,
        "notify" => {
            let title = option(arguments, "--title").unwrap_or_else(|| "Muxtrix".into());
            let body = option(arguments, "--body")
                .or_else(|| {
                    arguments
                        .get(1)
                        .filter(|value| !value.starts_with('-'))
                        .cloned()
                })
                .ok_or_else(|| "notify requires --body <text> or a positional body".to_owned())?;
            ControlRequest::Notify {
                title,
                body,
                pane_id: option(arguments, "--pane").or_else(pane_from_environment),
            }
        }
        "split" => ControlRequest::Split {
            direction: match arguments.get(1).map(String::as_str) {
                Some("right") => SplitDirection::Right,
                Some("down") => SplitDirection::Down,
                _ => return Err("split requires right or down".into()),
            },
        },
        "launch" => ControlRequest::LaunchAgent {
            agent: Agent::from_str(
                arguments
                    .get(1)
                    .ok_or_else(|| "launch requires codex, claude, or pi".to_owned())?,
            )
            .map_err(|error| error.to_string())?,
        },
        "focus" => ControlRequest::Focus {
            pane_id: arguments
                .get(1)
                .cloned()
                .ok_or_else(|| "focus requires a pane id".to_owned())?,
        },
        "close" => ControlRequest::Close {
            pane_id: option(arguments, "--pane").or_else(pane_from_environment),
        },
        "send" => ControlRequest::SendText {
            text: arguments
                .get(1)
                .cloned()
                .ok_or_else(|| "send requires text".to_owned())?,
            pane_id: option(arguments, "--pane"),
        },
        "capture" => ControlRequest::Capture {
            pane_id: option(arguments, "--pane"),
        },
        "panes" => ControlRequest::ListPanes,
        _ => return Err(usage()),
    };

    let environment_pane = pane_from_environment();
    let route_pane = request_pane_id(&request).or(environment_pane.as_deref());
    let endpoint = Endpoint::discover_for_pane(route_pane).map_err(|error| error.to_string())?;
    let response = send_request(&endpoint, &request).map_err(|error| error.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?
    );
    if response.ok {
        Ok(())
    } else {
        Err(response
            .message
            .unwrap_or_else(|| "Muxtrix rejected the request".into()))
    }
}

fn run_hooks(arguments: &[String]) -> Result<(), String> {
    validate_option_values(arguments, &["--scope", "--project", "--hook-command"])?;
    let action = match arguments.first().map(String::as_str) {
        Some("status") => None,
        Some("add") => Some(HookAction::Add),
        Some("remove") => Some(HookAction::Remove),
        Some("re-add" | "reinstall") => Some(HookAction::ReAdd),
        _ => return Err(hooks_usage()),
    };
    let agent_argument = arguments
        .get(1)
        .filter(|argument| !argument.starts_with('-'))
        .map_or("all", String::as_str);
    let agents: Vec<Agent> = if agent_argument == "all" {
        Agent::ALL.to_vec()
    } else {
        vec![Agent::from_str(agent_argument).map_err(|error| error.to_string())?]
    };
    let scope = option(arguments, "--scope")
        .map_or(Ok(HookScope::User), |scope| HookScope::from_str(&scope))
        .map_err(|error| error.to_string())?;
    let named_executable = option(arguments, "--hook-command").map(PathBuf::from);
    let executable = named_executable
        .clone()
        .map_or_else(std::env::current_exe, Ok)
        .map_err(|error| error.to_string())?;
    let mut manager = HookManager::discover(executable).map_err(|error| error.to_string())?;
    if named_executable.is_some() {
        // An operator naming the command may be provisioning another
        // environment's hooks — a WSL home from Windows, or a machine whose
        // install lands later. That path is theirs to vouch for.
        manager = manager.with_named_executable();
    }
    if let Some(project) = option(arguments, "--project") {
        manager = manager.project(PathBuf::from(project));
    }

    for agent in agents {
        if let Some(action) = action {
            let result = manager
                .apply(agent, scope, action)
                .map_err(|error| error.to_string())?;
            println!("{}", result.message);
            println!("  target: {}", result.status.target.display());
            println!("  managed entries: {}", result.status.managed_entries);
        } else {
            let status = manager
                .status(agent, scope)
                .map_err(|error| error.to_string())?;
            println!(
                "{} {}: {} ({} entries)\n  target: {}\n  recovery backup: {}",
                scope,
                agent,
                if status.installed {
                    "installed"
                } else {
                    "not installed"
                },
                status.managed_entries,
                status.target.display(),
                if status.backup_available {
                    "available"
                } else {
                    "none"
                }
            );
            // Naming the reason is the difference between a status a person
            // can act on and one they have to go diagnose.
            if status.unreachable_entries > 0 {
                println!(
                    "  needs repair: {} entries call a muxtrixctl that is not on disk",
                    status.unreachable_entries
                );
            }
        }
    }
    Ok(())
}

fn run_hook_event(arguments: &[String]) {
    let Some(agent) = option(arguments, "--agent") else {
        return;
    };
    let Some(state) = option(arguments, "--state").and_then(|state| parse_state(&state)) else {
        return;
    };
    let Some(pane_id) = pane_from_environment() else {
        return;
    };
    let mut input = String::new();
    let _ = std::io::stdin()
        .take(1024 * 1024)
        .read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("Lifecycle");
    if is_claude(&agent) {
        // Claude Code's state is decided by the app from the full payload
        // and the harness's own session record, not by the installed
        // command's `--state`.
        let mut hook = ClaudeHook::from_payload(&payload, event);
        hook.parent_process_id = parent_process_id();
        hook.sent_at_ms = now_ms();
        let request = ControlRequest::ClaudeHook {
            pane_id: Some(pane_id.clone()),
            hook,
        };
        if let Ok(endpoint) = Endpoint::discover_for_pane(Some(&pane_id)) {
            let _ = send_request(&endpoint, &request);
        }
        return;
    }
    if !event_changes_pane_state(event) {
        return;
    }
    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{} · {event}", agent_display_name(&agent)));
    let body = payload
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("last_assistant_message")
                .and_then(Value::as_str)
        })
        .map(short_body)
        .unwrap_or_else(|| default_body(&agent, state).into());
    let endpoint = Endpoint::discover_for_pane(Some(&pane_id)).ok();
    let request = ControlRequest::AgentEvent {
        agent,
        state,
        event: Some(event.to_owned()),
        title,
        body,
        pane_id: Some(pane_id),
        session_id: payload
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        cwd: payload
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_owned),
    };
    if let Some(endpoint) = endpoint {
        let _ = send_request(&endpoint, &request);
    }
}

fn is_claude(agent: &str) -> bool {
    matches!(agent, "claude" | "claude-code")
}

#[cfg(unix)]
fn parent_process_id() -> Option<u32> {
    Some(std::os::unix::process::parent_id())
}

#[cfg(not(unix))]
fn parent_process_id() -> Option<u32> {
    None
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Helper-agent lifecycle events describe progress inside a running turn —
/// a teammate or subagent finishing does not mean the session needs the
/// user's input or is done. Filtering here keeps the installed hook set
/// stable across versions, so no hook repair is required.
fn event_changes_pane_state(event: &str) -> bool {
    !matches!(event, "TeammateIdle" | "SubagentStop")
}

fn request_pane_id(request: &ControlRequest) -> Option<&str> {
    match request {
        ControlRequest::Notify { pane_id, .. }
        | ControlRequest::AgentEvent { pane_id, .. }
        | ControlRequest::ClaudeHook { pane_id, .. }
        | ControlRequest::Close { pane_id }
        | ControlRequest::SendText { pane_id, .. }
        | ControlRequest::Capture { pane_id } => pane_id.as_deref(),
        ControlRequest::Focus { pane_id } => Some(pane_id),
        ControlRequest::Ping
        | ControlRequest::LaunchAgent { .. }
        | ControlRequest::Split { .. }
        | ControlRequest::ListPanes
        | ControlRequest::E2eStatus
        | ControlRequest::Quit => None,
    }
}

fn option(arguments: &[String], name: &str) -> Option<String> {
    arguments
        .windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}

fn validate_option_values(arguments: &[String], names: &[&str]) -> Result<(), String> {
    for (index, argument) in arguments.iter().enumerate() {
        if names.contains(&argument.as_str())
            && arguments
                .get(index + 1)
                .is_none_or(|value| value.starts_with("--"))
        {
            return Err(format!("{argument} requires a value on the same command"));
        }
    }
    Ok(())
}

fn pane_from_environment() -> Option<String> {
    std::env::var("MUXTRIX_PANE_ID").ok()
}

fn parse_state(value: &str) -> Option<AgentState> {
    match value {
        "idle" => Some(AgentState::Idle),
        "running" => Some(AgentState::Running),
        "waiting" => Some(AgentState::Waiting),
        "completed" => Some(AgentState::Completed),
        "failed" | "error" => Some(AgentState::Failed),
        "stopped" => Some(AgentState::Stopped),
        _ => None,
    }
}

fn default_body(agent: &str, state: AgentState) -> &'static str {
    let _ = agent;
    match state {
        AgentState::Idle => "Agent is ready for input",
        AgentState::Running => "Agent is running",
        AgentState::Waiting => "Agent needs attention",
        AgentState::Completed => "Agent completed a turn",
        AgentState::Failed => "Agent reported an error",
        AgentState::Stopped => "Agent session ended",
    }
}

fn short_body(body: &str) -> String {
    let truncated = body.chars().count() > 240;
    let mut body: String = body.chars().take(240).collect();
    if truncated {
        body.push('…');
    }
    body
}

fn agent_display_name(agent: &str) -> &str {
    match agent {
        "codex" => "Codex",
        "claude" => "Claude Code",
        "pi" | "omp" | "oh-my-pi" => "Oh My Pi",
        _ => agent,
    }
}

fn usage() -> String {
    "usage: muxtrixctl [--version] <ping|notify|launch|split|focus|close|send|capture|panes|hooks> ..."
        .into()
}

fn hooks_usage() -> String {
    "usage: muxtrixctl hooks <status|add|remove|re-add> [codex|claude|pi|all] [--scope user|project] [--project PATH] [--hook-command PATH]".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangling_options_fail_instead_of_falling_back_to_the_focused_pane() {
        let arguments = vec!["send".into(), "echo test".into(), "--pane".into()];
        let error = run(&arguments).expect_err("a missing pane id should be rejected");
        assert_eq!(error, "--pane requires a value on the same command");
    }

    #[test]
    fn version_flag_is_a_complete_command() {
        assert_eq!(run(&["--version".into()]), Ok(()));
        assert_eq!(run(&["-V".into()]), Ok(()));
    }

    #[test]
    fn helper_lifecycle_events_never_repaint_a_working_pane() {
        // A teammate going idle or a subagent stopping happens inside a
        // running turn; painting "waiting" or "completed" then is false.
        assert!(!event_changes_pane_state("TeammateIdle"));
        assert!(!event_changes_pane_state("SubagentStop"));
        assert!(event_changes_pane_state("UserPromptSubmit"));
        assert!(event_changes_pane_state("Notification"));
        assert!(event_changes_pane_state("PermissionRequest"));
        assert!(event_changes_pane_state("Stop"));
        assert!(event_changes_pane_state("Lifecycle"));
    }

    #[test]
    fn idle_hook_state_describes_an_agent_ready_for_input() {
        assert_eq!(parse_state("idle"), Some(AgentState::Idle));
        assert_eq!(
            default_body("codex", AgentState::Idle),
            "Agent is ready for input"
        );
    }
}
