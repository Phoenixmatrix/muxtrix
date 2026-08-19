//! Conservative, screen-authoritative state detection for interactive agents.
//!
//! Hooks identify sessions and turn boundaries, but permission hooks run before
//! an automatic reviewer has decided whether a person is needed. Only positive
//! evidence in the live terminal frame is therefore allowed to produce
//! `Waiting`.
//!
//! Portions of the Claude prompt-state detection were adapted from Herdr
//! (https://github.com/herdrdev/herdr) and modified for Muxtrix. Herdr is
//! licensed under Apache-2.0; see THIRD_PARTY_NOTICES.md.

use muxtrix_terminal::GridSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenState {
    Waiting,
    Running,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Classification {
    pub(crate) state: ScreenState,
    pub(crate) rule: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identification {
    pub(crate) agent: &'static str,
    pub(crate) classification: Option<Classification>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PiTitle<'a> {
    state: Option<ScreenState>,
    label: &'a str,
    rule: &'static str,
}

/// Codex and Claude approval hooks are advisory because their harnesses may
/// auto-resolve them. Pi's approval events are exact, so only those two agents
/// require a visible blocker before entering Waiting.
pub(crate) fn requires_screen_confirmed_wait(agent: &str) -> bool {
    matches!(
        agent.to_ascii_lowercase().as_str(),
        "codex" | "claude" | "claude-code"
    )
}

/// True while the pane is showing Claude Code's Agents view — the roster of
/// interactive and background sessions reached with `←` on an empty prompt,
/// `/background`, or `claude agents`.
///
/// The pane is then projecting a fleet rather than its own conversation, so
/// its own lifecycle state is neither visible nor meaningful. Callers roll the
/// roster up instead of reading this frame as one agent.
pub(crate) fn agents_view(agent: &str, snapshot: &GridSnapshot) -> bool {
    let rows = snapshot
        .rows
        .iter()
        .map(|row| row.trim_end().to_owned())
        .collect::<Vec<_>>();
    is_agents_view(agent, snapshot.title.as_deref().unwrap_or_default(), &rows)
}

fn is_agents_view(agent: &str, title: &str, rows: &[String]) -> bool {
    matches!(
        agent.to_ascii_lowercase().as_str(),
        "claude" | "claude-code"
    ) && is_agents_view_frame(title, rows)
}

fn is_agents_view_frame(title: &str, rows: &[String]) -> bool {
    if is_agents_view_title(title) {
        return true;
    }
    // The roster's own chrome, for terminals where the harness title is
    // suppressed. Both keys are specific to the roster — the conversation
    // offers `← for agents` and a composer that names no session — and the
    // roster's footer alternates between `enter to expand` and
    // `enter to collapse`, so neither verb may be required.
    let footer = recent_text(rows, 5).to_ascii_lowercase();
    footer.contains("ctrl+x to delete all") || footer.contains("describe a task for a new session")
}

/// Claude Code publishes exactly one of `claude agents` or
/// `<n> awaiting input · claude agents` while the roster is on screen.
fn is_agents_view_title(title: &str) -> bool {
    let title = title.trim().to_ascii_lowercase();
    title == "claude agents" || title.ends_with("· claude agents")
}

/// True for titles that describe the harness's current *view* rather than the
/// session's work: the roster's own titles, and the `current session` label
/// Claude Code emits when returning from the roster.
///
/// These must not become pane identity. Toggling into the roster and back
/// would otherwise rename a fleet row twice and leave it stuck on the label of
/// whichever row the roster happened to highlight.
pub(crate) fn is_view_chrome_title(agent: &str, title: &str) -> bool {
    if !matches!(
        agent.to_ascii_lowercase().as_str(),
        "claude" | "claude-code"
    ) {
        return false;
    }
    is_agents_view_title(title) || title.trim().eq_ignore_ascii_case("current session")
}

/// Removes harness-owned animated progress glyphs while retaining the title's
/// actual task or session copy. OSC titles feed both pane identity and native
/// window chrome, where publishing every spinner frame creates visual jitter.
pub(crate) fn stable_title(agent: &str, title: &str) -> String {
    let agent = agent.to_ascii_lowercase();
    if matches!(agent.as_str(), "pi" | "omp" | "oh-my-pi")
        && let Some(title) = parse_pi_title(title)
    {
        return title.label.to_owned();
    }
    title
        .split_whitespace()
        .enumerate()
        .filter_map(|(index, part)| {
            let is_progress = match agent.as_str() {
                "codex" => is_codex_spinner(part),
                "claude" | "claude-code" => index == 0 && is_claude_spinner(part),
                _ => false,
            };
            (!is_progress).then_some(part)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn classify(agent: &str, snapshot: &GridSnapshot) -> Option<Classification> {
    let title = snapshot.title.as_deref().unwrap_or_default();
    if ["pi", "omp", "oh-my-pi"]
        .iter()
        .any(|candidate| agent.eq_ignore_ascii_case(candidate))
    {
        return classify_pi(title);
    }
    let rows = snapshot
        .rows
        .iter()
        .map(|row| row.trim_end().to_owned())
        .collect::<Vec<_>>();
    classify_text(agent, title, &rows)
}

/// Recovers agent identity from a replayed screen when an older session has no
/// durable pane identity yet. Every signature here is agent-specific; generic
/// nonempty titles and ambiguous spinner glyphs are deliberately insufficient.
pub(crate) fn identify(snapshot: &GridSnapshot) -> Option<Identification> {
    let title = snapshot.title.as_deref().unwrap_or_default();
    if let Some(identification) = pi_identification(title) {
        return Some(identification);
    }
    let rows = snapshot
        .rows
        .iter()
        .map(|row| row.trim_end().to_owned())
        .collect::<Vec<_>>();
    identify_text(title, &rows)
}

/// Whether the live frame carries `agent`'s own chrome — the same
/// agent-specific signatures `identify` recovers identity from. Unknown agent
/// names have no signature and therefore never match.
pub(crate) fn carries_signature(agent: &str, snapshot: &GridSnapshot) -> bool {
    let title = snapshot.title.as_deref().unwrap_or_default();
    let rows = snapshot
        .rows
        .iter()
        .map(|row| row.trim_end().to_owned())
        .collect::<Vec<_>>();
    carries_signature_text(agent, title, &rows)
}

fn carries_signature_text(agent: &str, title: &str, rows: &[String]) -> bool {
    match agent.to_ascii_lowercase().as_str() {
        "claude" | "claude-code" => has_claude_signature(title, rows),
        "codex" => has_codex_signature(title, rows),
        "pi" | "omp" | "oh-my-pi" => parse_pi_title(title).is_some(),
        _ => false,
    }
}

fn identify_text(title: &str, rows: &[String]) -> Option<Identification> {
    if has_claude_signature(title, rows) {
        return Some(Identification {
            agent: "claude",
            // The Agents view intentionally has no conversation state, but
            // its chrome still identifies the pane so the roster can render.
            classification: classify_claude(title, rows),
        });
    }
    if has_codex_signature(title, rows) {
        return classify_codex(title, rows).map(|classification| Identification {
            agent: "codex",
            classification: Some(classification),
        });
    }
    if let Some(identification) = pi_identification(title) {
        return Some(identification);
    }
    None
}

fn pi_identification(title: &str) -> Option<Identification> {
    let title = parse_pi_title(title)?;
    Some(Identification {
        agent: "pi",
        classification: title.state.map(|state| Classification {
            state,
            rule: title.rule,
        }),
    })
}

fn has_claude_signature(title: &str, rows: &[String]) -> bool {
    if is_agents_view_frame(title, rows) || title.starts_with("✳ ") {
        return true;
    }
    let stable = stable_title("claude", title);
    if stable.eq_ignore_ascii_case("claude") || stable.eq_ignore_ascii_case("claude code") {
        return true;
    }
    let recent = recent_text(rows, 8).to_ascii_lowercase();
    is_live_prompt_box(rows, &recent)
        || recent.contains("← for agents")
        || (recent.contains("shift+tab to cycle") && recent.contains("auto mode"))
}

fn has_codex_signature(title: &str, rows: &[String]) -> bool {
    if title.contains("Action Required") {
        return true;
    }
    let stable = stable_title("codex", title);
    if stable.eq_ignore_ascii_case("codex") || stable.eq_ignore_ascii_case("codex cli") {
        return true;
    }
    let recent = recent_text(rows, 20).to_ascii_lowercase();
    has_recent_codex_prompt(rows)
        || recent.contains("press enter to confirm or esc to cancel")
        || recent.contains("enter to submit answer")
        || recent.contains("enter to submit all")
        || recent.contains("allow command?")
        || recent.contains("do you trust the contents of this directory?")
        || rows
            .iter()
            .rev()
            .filter(|row| !row.trim().is_empty())
            .take(3)
            .any(|row| {
                let row = row.trim_start();
                (row.starts_with("• Working (") || row.starts_with("◦ Working ("))
                    && row.contains("esc to interrupt")
            })
}

fn has_recent_codex_prompt(rows: &[String]) -> bool {
    rows.iter().rev().take(8).any(|row| {
        let row = row.trim();
        row == "›" || row.starts_with("› ")
    })
}

fn classify_text(agent: &str, title: &str, rows: &[String]) -> Option<Classification> {
    let agent = agent.to_ascii_lowercase();
    match agent.as_str() {
        "codex" => classify_codex(title, rows),
        "claude" | "claude-code" => classify_claude(title, rows),
        "pi" | "omp" | "oh-my-pi" => classify_pi(title),
        _ => None,
    }
}

fn classify_pi(title: &str) -> Option<Classification> {
    let title = parse_pi_title(title)?;
    title.state.map(|state| Classification {
        state,
        rule: title.rule,
    })
}

/// OMP 17.3.4 publishes a documented state separator in every authoritative
/// terminal title: `>` idle, `!` attention, a Braille frame while working, and
/// a static `:` while working under ConPTY. `π: label` has no spaces and means
/// title-state reporting is disabled, so it identifies Pi without inventing a
/// lifecycle state.
fn parse_pi_title(title: &str) -> Option<PiTitle<'_>> {
    let title = title.trim();
    if title == "π" {
        return Some(PiTitle {
            state: None,
            label: "",
            rule: "pi.osc_title_disabled",
        });
    }
    if let Some(label) = title.strip_prefix("π:") {
        return Some(PiTitle {
            state: None,
            label: label.trim(),
            rule: "pi.osc_title_disabled",
        });
    }
    let state_title = title.strip_prefix("π ")?;
    let (separator, label) = state_title
        .split_once(' ')
        .map_or((state_title, ""), |(separator, label)| {
            (separator, label.trim())
        });
    let (state, rule) = match separator {
        ">" => (ScreenState::Idle, "pi.osc_title_idle"),
        "!" => (ScreenState::Waiting, "pi.osc_title_attention"),
        ":" => (ScreenState::Running, "pi.osc_title_working"),
        spinner if is_codex_spinner(spinner) => (ScreenState::Running, "pi.osc_title_spinner"),
        _ => return None,
    };
    Some(PiTitle {
        state: Some(state),
        label,
        rule,
    })
}

fn classify_codex(title: &str, rows: &[String]) -> Option<Classification> {
    if title.contains("Action Required") {
        return classification(ScreenState::Waiting, "codex.osc_title_action_required");
    }
    if title_has_codex_spinner(title) {
        return classification(ScreenState::Running, "codex.osc_title_spinner");
    }

    let live_rows = after_last_codex_prompt(rows);
    let recent = recent_text(live_rows, 20);
    let recent_lower = recent.to_ascii_lowercase();
    if is_codex_transcript_viewer(&recent_lower) {
        return None;
    }
    if recent_lower.contains("do you trust the contents of this directory?")
        && rows
            .iter()
            .take(20)
            .any(|row| row.trim_start().starts_with("> You are in "))
    {
        return classification(ScreenState::Waiting, "codex.trust_directory");
    }
    if [
        "press enter to confirm or esc to cancel",
        "enter to submit answer",
        "enter to submit all",
        "allow command?",
    ]
    .iter()
    .any(|needle| recent_lower.contains(needle))
    {
        return classification(ScreenState::Waiting, "codex.live_strong_blocker");
    }
    if rows
        .iter()
        .rev()
        .filter(|row| !row.trim().is_empty())
        .take(3)
        .any(|row| {
            let row = row.trim_start();
            (row.starts_with("• Working (") || row.starts_with("◦ Working ("))
                && row.contains("esc to interrupt")
                && !row.contains("Conversation interrupted")
        })
    {
        return classification(ScreenState::Running, "codex.screen_working");
    }
    if has_recent_codex_prompt(rows) {
        return classification(ScreenState::Idle, "codex.live_prompt");
    }
    if !title.trim().is_empty() {
        return classification(ScreenState::Idle, "codex.osc_title_idle");
    }
    None
}

fn classify_claude(title: &str, rows: &[String]) -> Option<Classification> {
    // The roster is a different surface, not a state of this conversation. It
    // carries its own prompt box and its own spinner-free title, so it must be
    // rejected before any rule below reads those as the session's own.
    if is_agents_view_frame(title, rows) {
        return None;
    }
    if title_has_claude_spinner(title) {
        return classification(ScreenState::Running, "claude.osc_title_spinner");
    }

    let recent = recent_text(rows, 24);
    let recent_lower = recent.to_ascii_lowercase();
    let form = recent_text(after_last_horizontal_rule(rows), 24);
    let form_lower = form.to_ascii_lowercase();
    let bottom = recent_text(painted_rows(rows), 5);
    let bottom_lower = bottom.to_ascii_lowercase();
    if bottom_lower.contains("showing detailed transcript")
        && ["ctrl+o", "ctrl+e", "↑↓ scroll", "? for shortcuts"]
            .iter()
            .any(|needle| bottom_lower.contains(needle))
    {
        return None;
    }
    if bottom_lower.contains("/btw") && bottom_lower.contains("esc to close") {
        return classification(ScreenState::Running, "claude.btw_overlay");
    }

    let has_confirm_navigation = form_lower.contains("enter to confirm")
        || (form_lower.contains("enter to select")
            && [
                "tab/arrow keys to navigate",
                "arrow keys to navigate",
                "arrows to navigate",
                "↑/↓ to navigate",
                "↑↓ to navigate",
            ]
            .iter()
            .any(|needle| form_lower.contains(needle)));
    if form_lower.contains("esc to cancel") && has_confirm_navigation {
        return classification(ScreenState::Waiting, "claude.live_blocked_form");
    }
    if recent_lower.contains("run a dynamic workflow?") && recent_lower.contains("esc to cancel") {
        return classification(ScreenState::Waiting, "claude.dynamic_workflow_prompt");
    }
    if recent_lower.contains("do you want to proceed?")
        && recent_lower.contains("esc to cancel")
        && recent_lower.lines().any(is_numbered_answer)
    {
        return classification(ScreenState::Waiting, "claude.permission_prompt");
    }
    // Claude Code prints `esc to interrupt` in its footer exactly while a
    // turn is loading, and it keeps the empty composer painted underneath
    // that footer the whole time. The footer therefore outranks the composer:
    // without it, a working session whose title carries no spinner — the
    // prefix is optional harness chrome — would read as idle from its own
    // prompt box. Every blocking rule above still wins over it.
    if bottom_lower.contains("esc to interrupt") {
        return classification(ScreenState::Running, "claude.footer_interrupt");
    }
    // Screen-visible idle, ranked below every blocking rule above so it can
    // never clear a wait that is still painted. This is the only idle evidence
    // that survives Claude Code emitting a non-`✳` title — notably the
    // `current session` title left behind after returning from the roster,
    // which otherwise persists until the next turn.
    if is_live_prompt_box(rows, &recent_lower) {
        return classification(ScreenState::Idle, "claude.live_prompt_box");
    }
    if title.starts_with("✳ ") {
        return classification(ScreenState::Idle, "claude.osc_title_idle");
    }
    None
}

/// The empty composer between the last two horizontal rules. A rendered prompt
/// box with no menu over it is positive evidence that the harness is waiting
/// for a new instruction rather than working.
fn is_live_prompt_box(rows: &[String], recent_lower: &str) -> bool {
    let body = prompt_box_body(rows);
    let has_prompt = body.iter().any(|row| {
        let row = row.trim_start();
        // `is_numbered_answer` matches lowercase, as its other caller supplies.
        row.starts_with('❯') && !is_numbered_answer(&row.to_ascii_lowercase())
    });
    has_prompt
        && ![
            "enter to select",
            "esc to cancel",
            "tab/arrow keys",
            "arrow keys to navigate",
            "↑/↓ to navigate",
            "↑↓ to navigate",
        ]
        .iter()
        .any(|needle| recent_lower.contains(needle))
}

fn classification(state: ScreenState, rule: &'static str) -> Option<Classification> {
    Some(Classification { state, rule })
}

fn recent_text(rows: &[String], count: usize) -> String {
    rows[rows.len().saturating_sub(count)..].join("\n")
}

/// The frame without its trailing blank rows, so footer rules read the last
/// painted lines rather than the empty bottom of a taller grid.
fn painted_rows(rows: &[String]) -> &[String] {
    let end = rows
        .iter()
        .rposition(|row| !row.trim().is_empty())
        .map_or(0, |index| index + 1);
    &rows[..end]
}

fn after_last_codex_prompt(rows: &[String]) -> &[String] {
    rows.iter()
        .rposition(|row| {
            let row = row.trim();
            row == "›" || row.starts_with("› ")
        })
        .map_or(rows, |index| &rows[index + 1..])
}

fn after_last_horizontal_rule(rows: &[String]) -> &[String] {
    rows.iter()
        .rposition(|row| is_horizontal_rule(row))
        .map_or(rows, |index| &rows[index + 1..])
}

/// The rows enclosed by the final pair of horizontal rules — Claude Code's
/// composer. Empty when the frame does not draw a closed box, so a partially
/// painted screen cannot be read as a prompt.
fn prompt_box_body(rows: &[String]) -> &[String] {
    let Some(close) = rows.iter().rposition(|row| is_horizontal_rule(row)) else {
        return &[];
    };
    rows[..close]
        .iter()
        .rposition(|row| is_horizontal_rule(row))
        .map_or(&[], |open| &rows[open + 1..close])
}

fn is_horizontal_rule(row: &str) -> bool {
    let row = row.trim();
    let rules = row
        .chars()
        .take_while(|character| *character == '─')
        .count();
    rules > 0 && (rules >= 3 || row.chars().all(|character| character == '─'))
}

fn is_codex_transcript_viewer(text: &str) -> bool {
    text.contains("↑/↓ to scroll")
        && text.contains("pgup/pgdn to")
        && text.contains("home/end to jump")
        && text.contains("q to quit")
        && (text.contains("esc to edit prev") || text.contains("esc/← to edit prev"))
}

fn title_has_codex_spinner(title: &str) -> bool {
    title.split_whitespace().any(is_codex_spinner)
}

fn title_has_claude_spinner(title: &str) -> bool {
    let Some(first) = title.chars().next() else {
        return false;
    };
    title.chars().nth(1) == Some(' ')
        && (('\u{2800}'..='\u{28ff}').contains(&first) || matches!(first, '◐' | '◑'))
}

fn is_codex_spinner(part: &str) -> bool {
    const SPINNERS: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    part.chars().count() == 1
        && part
            .chars()
            .next()
            .is_some_and(|character| SPINNERS.contains(&character))
}

fn is_claude_spinner(part: &str) -> bool {
    part.chars().count() == 1
        && part.chars().next().is_some_and(|character| {
            ('\u{2800}'..='\u{28ff}').contains(&character) || matches!(character, '◐' | '◑')
        })
}

fn is_numbered_answer(line: &str) -> bool {
    let line = line.trim_start().trim_start_matches('❯').trim_start();
    ["1. yes", "2. yes", "2. no", "3. no"]
        .iter()
        .any(|answer| line.starts_with(answer))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn codex_requires_visible_blocking_evidence() {
        assert_eq!(
            classify_text("codex", "Action Required", &[]),
            classification(ScreenState::Waiting, "codex.osc_title_action_required")
        );
        assert_eq!(
            classify_text(
                "codex",
                "Codex",
                &rows(&["Allow command?", "Press enter to confirm or esc to cancel"]),
            ),
            classification(ScreenState::Waiting, "codex.live_strong_blocker")
        );
        assert_eq!(
            classify_text("codex", "", &rows(&["Should I proceed?"])),
            None
        );
    }

    #[test]
    fn codex_working_title_outranks_old_prompt_text() {
        assert_eq!(
            classify_text(
                "codex",
                "⠹ Codex",
                &rows(&["Allow command?", "Press enter to confirm or esc to cancel"]),
            ),
            classification(ScreenState::Running, "codex.osc_title_spinner")
        );
    }

    #[test]
    fn codex_transcript_viewer_preserves_the_previous_state() {
        assert_eq!(
            classify_text(
                "codex",
                "",
                &rows(&[
                    "↑/↓ to scroll · pgup/pgdn to page · home/end to jump",
                    "esc to edit prev · q to quit",
                ]),
            ),
            None
        );
    }

    #[test]
    fn claude_distinguishes_working_idle_and_blocked_surfaces() {
        assert_eq!(
            classify_text("claude", "◐ Claude", &[]),
            classification(ScreenState::Running, "claude.osc_title_spinner")
        );
        assert_eq!(
            classify_text("claude", "✳ Claude", &[]),
            classification(ScreenState::Idle, "claude.osc_title_idle")
        );
        assert_eq!(
            classify_text(
                "claude",
                "Claude",
                &rows(&[
                    "Do you want to proceed?",
                    "❯ 1. Yes",
                    "  2. No",
                    "Esc to cancel",
                ]),
            ),
            classification(ScreenState::Waiting, "claude.permission_prompt")
        );
    }

    /// Verbatim tail of a Claude Code 2.1.235 frame mid-turn: the spinner
    /// line, the still-painted empty composer, and the loading footer.
    fn working_frame() -> Vec<String> {
        rows(&[
            "✶ Sock-hopping… (1m 28s · ↓ 4.9k tokens)",
            "",
            "────────────────────────────────────────────────────────",
            "❯",
            "────────────────────────────────────────────────────────",
            "  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt · ← for agents",
        ])
    }

    #[test]
    fn claude_loading_footer_outranks_the_empty_composer() {
        // A spinner-free title cannot make a working session idle: the
        // composer stays painted while Claude Code works, so the footer's
        // interrupt hint is what says the turn is still running.
        assert_eq!(
            classify_text("claude", "Fleet sidebar Running to Idle", &working_frame()),
            classification(ScreenState::Running, "claude.footer_interrupt")
        );
        assert_eq!(
            classify_text("claude", "", &working_frame()),
            classification(ScreenState::Running, "claude.footer_interrupt")
        );
        // The same composer without the interrupt hint is the idle prompt.
        assert_eq!(
            classify_text(
                "claude",
                "Fleet sidebar Running to Idle",
                &conversation_frame("❯")
            ),
            classification(ScreenState::Idle, "claude.live_prompt_box")
        );
        // A painted dialog still wins over the loading footer.
        let mut blocked = rows(&[
            "Do you want to proceed?",
            "❯ 1. Yes",
            "  2. No",
            "Esc to cancel",
        ]);
        blocked.push("  ⏵⏵ auto mode on (shift+tab to cycle) · esc to interrupt".into());
        assert_eq!(
            classify_text("claude", "Claude", &blocked),
            classification(ScreenState::Waiting, "claude.permission_prompt")
        );
    }

    #[test]
    fn signatures_are_agent_specific() {
        let working = working_frame();
        assert!(carries_signature_text(
            "claude",
            "◑ Fleet sidebar Running to Idle",
            &working
        ));
        assert!(carries_signature_text(
            "claude",
            "current session",
            &conversation_frame("❯")
        ));
        // The frame of one agent never vouches for another: a Pi identity
        // needs Pi's own title, and Codex its own prompt or blocker.
        assert!(!carries_signature_text(
            "pi",
            "◑ Fleet sidebar Running to Idle",
            &working
        ));
        assert!(!carries_signature_text(
            "codex",
            "◑ Fleet sidebar Running to Idle",
            &working
        ));
        assert!(carries_signature_text("oh-my-pi", "π : Fix Pi state", &[]));
        assert!(carries_signature_text("omp", "π: Fix Pi state", &[]));
        assert!(!carries_signature_text("claude", "π : Fix Pi state", &[]));
        assert!(carries_signature_text(
            "codex",
            "Fix resume status",
            &rows(&["› "])
        ));
        assert!(!carries_signature_text(
            "build",
            "◑ build watcher",
            &working
        ));
    }

    #[test]
    fn pi_title_state_distinguishes_idle_working_and_attention() {
        assert_eq!(
            classify_text("pi", "π > Fix Pi state", &[]),
            classification(ScreenState::Idle, "pi.osc_title_idle")
        );
        assert_eq!(
            classify_text("omp", "π ! Fix Pi state", &[]),
            classification(ScreenState::Waiting, "pi.osc_title_attention")
        );
        assert_eq!(
            classify_text("oh-my-pi", "π : Fix Pi state", &[]),
            classification(ScreenState::Running, "pi.osc_title_working")
        );
        for spinner in ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"] {
            assert_eq!(
                classify_text("pi", &format!("π {spinner} Fix Pi state"), &[]),
                classification(ScreenState::Running, "pi.osc_title_spinner")
            );
        }
        assert_eq!(classify_text("pi", "π: Fix Pi state", &[]), None);
        assert_eq!(classify_text("pi", "π ? Fix Pi state", &[]), None);
    }

    #[test]
    fn animated_progress_glyphs_do_not_change_harness_titles() {
        for title in [
            "⠋ Fix window titles",
            "⠙ Fix window titles",
            "⠹ Fix window titles",
        ] {
            assert_eq!(stable_title("codex", title), "Fix window titles");
        }
        assert_eq!(
            stable_title("codex", "Fix ⠸ window titles"),
            "Fix window titles"
        );
        assert_eq!(
            stable_title("claude", "◐ Refine window titles"),
            "Refine window titles"
        );
        assert_eq!(
            stable_title("claude", "◑ Refine window titles"),
            "Refine window titles"
        );
        assert_eq!(stable_title("claude", "✳ Claude"), "✳ Claude");
        assert_eq!(
            stable_title("shell", "⠋ package update"),
            "⠋ package update"
        );
        for title in [
            "π > Fix Pi state",
            "π ! Fix Pi state",
            "π : Fix Pi state",
            "π ⠋ Fix Pi state",
            "π ⠏ Fix Pi state",
            "π: Fix Pi state",
        ] {
            assert_eq!(stable_title("pi", title), "Fix Pi state");
        }
    }

    /// Verbatim tail of a Claude Code 2.1.229 conversation frame.
    fn conversation_frame(prompt: &str) -> Vec<String> {
        rows(&[
            "─────────────────────────────── agents view detection ──",
            prompt,
            "────────────────────────────────────────────────────────",
            "  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents",
        ])
    }

    /// Verbatim tail of a Claude Code 2.1.229 Agents view frame, transcribed
    /// from a live roster running inside a pane.
    fn agents_view_frame() -> Vec<String> {
        rows(&[
            "────────────────────────────────────────────────────────",
            "❯ describe a task for a new session",
            "────────────────────────────────────────────────────────",
            "  enter to collapse · ctrl+x to delete all · ? for shortcuts",
        ])
    }

    #[test]
    fn agents_view_is_recognized_from_either_title_or_footer() {
        assert!(is_agents_view("claude", "claude agents", &[]));
        assert!(is_agents_view(
            "claude",
            "1 awaiting input · claude agents",
            &[]
        ));
        // Harness titles can be suppressed; the roster footer still identifies it.
        assert!(is_agents_view("claude", "", &agents_view_frame()));
        // The footer's first verb flips with the list's own expansion, so the
        // signature may not depend on it.
        assert!(is_agents_view(
            "claude",
            "",
            &rows(&["  enter to expand · ctrl+x to delete all · ? for shortcuts"])
        ));
        assert!(!is_agents_view("claude", "✳ Ship the fleet row", &[]));
        assert!(!is_agents_view("codex", "claude agents", &[]));
        // A conversation is never the roster, however recently it mentioned it.
        assert!(!is_agents_view(
            "claude",
            "",
            &conversation_frame("❯ open claude agents")
        ));
    }

    #[test]
    fn agents_view_never_reports_the_panes_own_state() {
        // The roster draws its own prompt box and its own title. Neither may be
        // read as this conversation being idle.
        assert_eq!(
            classify_text(
                "claude",
                "1 awaiting input · claude agents",
                &agents_view_frame()
            ),
            None
        );
        assert_eq!(
            classify_text("claude", "claude agents", &agents_view_frame()),
            None
        );
    }

    #[test]
    fn returning_from_the_agents_view_recovers_idle_without_an_idle_title() {
        // Claude Code emits `current session` on the way back and does not
        // repaint `✳ …` until the next turn. The prompt box is the only
        // evidence available in that window.
        assert_eq!(
            classify_text("claude", "current session", &conversation_frame("❯ ")),
            classification(ScreenState::Idle, "claude.live_prompt_box")
        );
    }

    #[test]
    fn replayed_agent_chrome_recovers_identity_without_hooks() {
        assert_eq!(
            identify_text("Fix resume status", &rows(&["› "])),
            Some(Identification {
                agent: "codex",
                classification: classification(ScreenState::Idle, "codex.live_prompt"),
            })
        );
        assert_eq!(
            identify_text(
                "current session",
                &conversation_frame("❯ continue the investigation")
            ),
            Some(Identification {
                agent: "claude",
                classification: classification(ScreenState::Idle, "claude.live_prompt_box"),
            })
        );
        assert_eq!(
            identify_text("claude agents", &agents_view_frame()),
            Some(Identification {
                agent: "claude",
                classification: None,
            })
        );
        assert_eq!(
            identify_text("π: status support", &[]),
            Some(Identification {
                agent: "pi",
                classification: None,
            })
        );
        assert_eq!(
            identify_text("π > status support", &[]),
            Some(Identification {
                agent: "pi",
                classification: classification(ScreenState::Idle, "pi.osc_title_idle"),
            })
        );
    }

    #[test]
    fn generic_terminal_chrome_cannot_invent_an_agent() {
        assert_eq!(identify_text("build watcher", &rows(&["$ "])), None);
        assert_eq!(
            identify_text("◐ build watcher", &rows(&["compiling"])),
            None
        );
    }

    #[test]
    fn a_visible_prompt_box_cannot_clear_a_painted_wait() {
        let mut frame = rows(&[
            "Do you want to proceed?",
            "❯ 1. Yes",
            "  2. No",
            "Esc to cancel",
        ]);
        frame.extend(conversation_frame("❯ "));
        assert_eq!(
            classify_text("claude", "current session", &frame),
            classification(ScreenState::Waiting, "claude.permission_prompt")
        );
    }

    #[test]
    fn a_numbered_answer_is_not_an_empty_composer() {
        assert_eq!(
            classify_text(
                "claude",
                "current session",
                &rows(&["────────", "❯ 1. Yes", "────────", "footer"]),
            ),
            None
        );
    }

    #[test]
    fn view_chrome_titles_never_become_pane_identity() {
        for title in [
            "claude agents",
            "3 awaiting input · claude agents",
            "current session",
        ] {
            assert!(is_view_chrome_title("claude", title), "{title}");
        }
        assert!(!is_view_chrome_title("claude", "✳ Ship the fleet row"));
        assert!(!is_view_chrome_title("codex", "current session"));
    }

    #[test]
    fn narrative_questions_do_not_create_attention() {
        assert_eq!(
            classify_text(
                "claude",
                "Claude",
                &rows(&["I considered: do you want to proceed? Then continued."]),
            ),
            None
        );
    }
}
