//! Claude Code's machine-wide structured session state, read from
//! `claude agents --json`.
//!
//! Background records roll up a pane showing Claude Code's Agents view.
//! Interactive records correct the ordinary pane each session owns when
//! optional TUI chrome is absent or still paints an idle composer. The
//! harness's on-screen tally cannot serve either job: it groups every session
//! awaiting a human with sessions that are merely idle.

use serde::Deserialize;

use crate::process::{
    HELPER_COMMAND_TIMEOUT, ProcessCancellation, command_output, console_command,
};

/// Ranked worst-first: a roster reports the single most important thing
/// happening inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RosterSignal {
    Idle,
    Completed,
    Working,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AgentsRoster {
    pub(crate) working: usize,
    pub(crate) blocked: usize,
    pub(crate) failed: usize,
    /// Finished sessions. The harness's own view calls these "completed", and
    /// they outrank a session that never started when the roster picks its
    /// signal — but both read as `idle` on screen, exactly as a single agent
    /// pane's completed turn does.
    pub(crate) completed: usize,
    pub(crate) idle: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InteractiveState {
    Idle,
    Working,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InteractiveSession {
    pub(crate) pid: Option<u32>,
    pub(crate) session_id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) state: InteractiveState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentsSnapshot {
    pub(crate) roster: AgentsRoster,
    pub(crate) interactive_sessions: Vec<InteractiveSession>,
}

impl AgentsRoster {
    /// Sessions sitting at their composer, finished or never started. The
    /// distinction is real to the roster and invisible to the reader, so every
    /// count it paints merges the two.
    const fn resting(self) -> usize {
        self.completed + self.idle
    }

    /// The most important state present, or `None` for an empty roster.
    pub(crate) const fn signal(self) -> Option<RosterSignal> {
        if self.failed > 0 {
            Some(RosterSignal::Failed)
        } else if self.blocked > 0 {
            Some(RosterSignal::Blocked)
        } else if self.working > 0 {
            Some(RosterSignal::Working)
        } else if self.completed > 0 {
            Some(RosterSignal::Completed)
        } else if self.idle > 0 {
            Some(RosterSignal::Idle)
        } else {
            None
        }
    }

    /// Trailing fleet-row state: the count behind the most important signal
    /// only. The row has one line, and a roster's worst state is the one worth
    /// its width.
    pub(crate) fn label(self) -> String {
        match self.signal() {
            Some(RosterSignal::Failed) => format!("{} failed", self.failed),
            Some(RosterSignal::Blocked) => format!("{} needs input", self.blocked),
            Some(RosterSignal::Working) => format!("{} working", self.working),
            Some(RosterSignal::Completed | RosterSignal::Idle) => {
                format!("{} idle", self.resting())
            }
            None => "No agents".into(),
        }
    }

    /// The full breakdown, for surfaces that carry a line of context rather
    /// than a state chip.
    pub(crate) fn activity(self) -> String {
        let parts = [
            (self.failed, "failed"),
            (self.blocked, "needs input"),
            (self.working, "working"),
            (self.resting(), "idle"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, noun)| format!("{count} {noun}"))
        .collect::<Vec<_>>();
        if parts.is_empty() {
            return "No agents running".into();
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Deserialize)]
struct RosterEntry {
    /// `background` for the sessions the Agents view lists, `interactive` for
    /// a Claude Code someone is typing into.
    #[serde(default)]
    kind: Option<String>,
    /// Present for every session: `idle` or `busy`.
    #[serde(default)]
    status: Option<String>,
    /// Present for background sessions, and richer than `status`: it separates
    /// `blocked` and `failed` from an ordinary idle wait.
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default, rename = "sessionId", alias = "session_id")]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

impl RosterEntry {
    /// Whether this session is one the roster stands for.
    ///
    /// The Agents view lists background sessions only, and every interactive
    /// Claude Code on the machine already has a fleet row of its own — the pane
    /// it is running in. Counting those here would both disagree with the
    /// screen the pane is showing and count the viewing session itself.
    /// Unknown kinds are kept: a harness that stops publishing the field should
    /// degrade to over-reporting, never to an empty roster.
    fn is_agent(&self) -> bool {
        !self
            .kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("interactive"))
    }

    fn interactive_session(&self) -> Option<InteractiveSession> {
        if !self
            .kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("interactive"))
        {
            return None;
        }
        let state = match self.status.as_deref() {
            Some("busy") => InteractiveState::Working,
            Some("idle") => InteractiveState::Idle,
            _ => return None,
        };
        Some(InteractiveSession {
            pid: self.pid,
            session_id: self.session_id.clone(),
            cwd: self.cwd.clone(),
            state,
        })
    }

    fn signal(&self) -> RosterSignal {
        match self.state.as_deref() {
            Some("blocked") => return RosterSignal::Blocked,
            Some("failed") => return RosterSignal::Failed,
            Some("working") => return RosterSignal::Working,
            Some("done") => return RosterSignal::Completed,
            _ => {}
        }
        match self.status.as_deref() {
            Some("busy") => RosterSignal::Working,
            _ => RosterSignal::Idle,
        }
    }
}

pub(crate) fn parse(json: &str) -> Result<AgentsSnapshot, String> {
    let entries: Vec<RosterEntry> =
        serde_json::from_str(json).map_err(|error| format!("unreadable agent roster: {error}"))?;
    let mut snapshot = AgentsSnapshot::default();
    for entry in entries {
        if let Some(session) = entry.interactive_session() {
            snapshot.interactive_sessions.push(session);
            continue;
        }
        if !entry.is_agent() {
            continue;
        }
        match entry.signal() {
            RosterSignal::Failed => snapshot.roster.failed += 1,
            RosterSignal::Blocked => snapshot.roster.blocked += 1,
            RosterSignal::Working => snapshot.roster.working += 1,
            RosterSignal::Completed => snapshot.roster.completed += 1,
            RosterSignal::Idle => snapshot.roster.idle += 1,
        }
    }
    Ok(snapshot)
}

/// The executable from a configured launch command, keeping any directory so a
/// pinned build is polled rather than whichever `claude` is on `PATH`.
fn executable(command: &str) -> Option<&str> {
    let mut words = command.split_whitespace();
    let mut executable = words.next()?;
    if executable.eq_ignore_ascii_case("env")
        || executable.eq_ignore_ascii_case("sudo")
        || executable.contains('=')
    {
        executable = words.find(|word| !word.contains('='))?;
    }
    Some(executable.trim_matches(['\'', '"'])).filter(|value| !value.is_empty())
}

/// Blocking: callers run this off the UI thread. `wsl_distribution` is `Some`
/// when a Windows Muxtrix pane runs in WSL; an empty value selects WSL's
/// default distribution.
pub(crate) fn load(
    claude_command: &str,
    wsl_distribution: Option<&str>,
) -> Result<AgentsSnapshot, String> {
    let program = executable(claude_command).ok_or("no Claude Code command is configured")?;
    #[cfg(target_os = "windows")]
    let mut command = if let Some(distribution) = wsl_distribution {
        let mut command = console_command("wsl.exe");
        if !distribution.trim().is_empty() {
            command.args(["--distribution", distribution.trim()]);
        }
        command.args(["--exec", program]);
        command
    } else {
        console_command(program)
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let _ = wsl_distribution;
        console_command(program)
    };
    command
        .args(["agents", "--json"])
        .stdin(std::process::Stdio::null());
    // This one is paced rather than one-shot, so an unbounded wait would not
    // just leak a thread: it would strand the harness — and the console
    // Windows gave it — with nothing left to notice.
    let output = command_output(
        &mut command,
        HELPER_COMMAND_TIMEOUT,
        &ProcessCancellation::default(),
    )
    .map_err(|error| format!("could not run `{program} agents --json`: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`{program} agents --json` failed: {}",
            output.status
        ));
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim shape of `claude agents --json` on 2.1.229.
    const SAMPLE: &str = r#"[
      {"pid":1,"cwd":"/a","kind":"interactive","sessionId":"s1","name":"one","status":"idle"},
      {"pid":2,"id":"b","cwd":"/b","kind":"background","sessionId":"s2","name":"two","status":"busy","state":"working"},
      {"pid":3,"id":"c","cwd":"/c","kind":"background","sessionId":"s3","name":"three","status":"idle","state":"done"}
    ]"#;

    #[test]
    fn a_real_roster_payload_counts_the_sessions_the_agents_view_lists() {
        // Interactive sessions are panes in their own right and the harness's
        // own view never lists them, so the roll-up must not either.
        let snapshot = parse(SAMPLE).expect("sample parses");
        let roster = snapshot.roster;
        assert_eq!(roster.working, 1);
        assert_eq!(roster.completed, 1);
        assert_eq!(roster.idle, 0);
        assert_eq!(roster.label(), "1 working");
        assert_eq!(roster.activity(), "1 working · 1 idle");
        assert_eq!(
            snapshot.interactive_sessions,
            vec![InteractiveSession {
                pid: Some(1),
                session_id: Some("s1".into()),
                cwd: Some("/a".into()),
                state: InteractiveState::Idle,
            }]
        );
    }

    /// Verbatim from this machine while the harness's own view read
    /// `0 awaiting input · 1 working · 3 completed`.
    #[test]
    fn the_roll_up_agrees_with_the_view_the_pane_is_showing() {
        let roster = parse(
            r#"[
              {"kind":"interactive","name":"muxtrix-83","status":"idle"},
              {"kind":"background","name":"a4246237","status":"idle","state":"done"},
              {"kind":"background","name":"polling","status":"busy","state":"working"},
              {"kind":"interactive","name":"muxtrix-a6","status":"idle"},
              {"kind":"background","name":"glyph","status":"idle","state":"done"},
              {"kind":"background","name":"lifecycle","status":"idle","state":"done"}
            ]"#,
        )
        .expect("parses");
        let roster = roster.roster;
        assert_eq!(roster.working, 1);
        assert_eq!(roster.completed, 3);
        assert_eq!(roster.activity(), "1 working · 3 idle");
    }

    #[test]
    fn the_worst_state_present_is_the_one_reported() {
        let roster = AgentsRoster {
            working: 3,
            blocked: 1,
            failed: 0,
            completed: 0,
            idle: 2,
        };
        assert_eq!(roster.signal(), Some(RosterSignal::Blocked));
        assert_eq!(roster.label(), "1 needs input");

        let failing = AgentsRoster {
            working: 3,
            blocked: 1,
            failed: 1,
            completed: 0,
            idle: 0,
        };
        assert_eq!(failing.signal(), Some(RosterSignal::Failed));
        assert_eq!(failing.label(), "1 failed");
    }

    #[test]
    fn a_working_roster_outranks_its_finished_members() {
        // The user's case: three running, one finished, one status reported.
        let roster = parse(
            r#"[{"status":"busy","state":"working"},{"status":"busy","state":"working"},
                {"status":"busy","state":"working"},{"status":"idle","state":"done"}]"#,
        )
        .expect("parses");
        let roster = roster.roster;
        assert_eq!(roster.label(), "3 working");
    }

    /// A fleet that has all finished keeps its own signal — the neutral pip a
    /// completed agent wears, not the quietest state there is — while the words
    /// beside it say what the reader sees: agents waiting at their composers.
    #[test]
    fn a_finished_fleet_keeps_its_signal_and_reads_as_idle() {
        let roster = parse(
            r#"[{"kind":"background","status":"idle","state":"done"},
                {"kind":"background","status":"idle","state":"done"}]"#,
        )
        .expect("parses");
        let roster = roster.roster;
        assert_eq!(roster.signal(), Some(RosterSignal::Completed));
        assert_eq!(roster.label(), "2 idle");
    }

    /// Finished and never-started sessions share one word, so they must share
    /// one count: two lines reading "4 idle · 2 idle" would be nonsense, and
    /// naming only half of the resting sessions would be untrue.
    #[test]
    fn resting_sessions_are_counted_together_under_one_word() {
        let roster = AgentsRoster {
            working: 0,
            blocked: 0,
            failed: 0,
            completed: 4,
            idle: 2,
        };
        assert_eq!(roster.label(), "6 idle");
        assert_eq!(roster.activity(), "6 idle");
    }

    #[test]
    fn blocked_and_failed_states_are_taken_from_the_session_not_its_status() {
        let roster =
            parse(r#"[{"status":"idle","state":"blocked"},{"status":"idle","state":"failed"}]"#)
                .expect("parses");
        let roster = roster.roster;
        assert_eq!(roster.blocked, 1);
        assert_eq!(roster.failed, 1);
        assert_eq!(roster.idle, 0);
    }

    #[test]
    fn an_empty_roster_reports_no_agents_rather_than_a_zero() {
        let roster = parse("[]").expect("parses");
        let roster = roster.roster;
        assert_eq!(roster.signal(), None);
        assert_eq!(roster.label(), "No agents");
        assert_eq!(roster.activity(), "No agents running");
    }

    #[test]
    fn an_unknown_state_falls_back_to_the_coarse_status() {
        let roster = parse(r#"[{"status":"busy","state":"something-new"}]"#)
            .expect("parses")
            .roster;
        assert_eq!(roster.working, 1);
    }

    #[test]
    fn malformed_output_is_an_error_rather_than_an_empty_roster() {
        assert!(parse("not json").is_err());
    }

    #[test]
    fn a_configured_command_keeps_its_path_and_drops_its_arguments() {
        assert_eq!(executable("claude"), Some("claude"));
        assert_eq!(executable("claude --model opus"), Some("claude"));
        assert_eq!(
            executable("/opt/bin/claude --continue"),
            Some("/opt/bin/claude")
        );
        assert_eq!(executable("env FOO=1 claude"), Some("claude"));
        assert_eq!(executable("   "), None);
    }
}
