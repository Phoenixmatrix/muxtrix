#![cfg(unix)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use libghostty_vt::TerminalOptions;
use muxtrix_platform::{LaunchPlan, PtySize};
use muxtrix_terminal::{GridSnapshot, LiveSession, LiveSessionEvent, Rgb, TerminalTheme};

fn options(size: PtySize) -> TerminalOptions {
    TerminalOptions {
        cols: size.cols,
        rows: size.rows,
        max_scrollback: 100,
    }
}

fn interactive_session() -> Result<LiveSession, Box<dyn std::error::Error>> {
    let size = PtySize {
        rows: 6,
        cols: 40,
        pixel_width: 400,
        pixel_height: 120,
    };
    let plan = LaunchPlan {
        executable: "/bin/sh".into(),
        arguments: vec![
            "-c".into(),
            "stty -echo; IFS= read -r line; printf '\\033[32mreply:%s\\033[0m\\n' \"$line\"; sleep 1".into(),
        ],
        working_directory: Some(PathBuf::from("/tmp")),
        environment: vec![("TERM".into(), "xterm-256color".into())],
    };
    Ok(LiveSession::spawn(plan, size, options(size))?)
}

fn wait_for_text(
    session: &LiveSession,
    needle: &str,
) -> Result<GridSnapshot, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match session.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveSessionEvent::Frame(snapshot)) if snapshot.text().contains(needle) => {
                return Ok(snapshot);
            }
            Ok(LiveSessionEvent::Frame(_)) => {}
            Ok(LiveSessionEvent::Notification(_)) => {}
            Ok(LiveSessionEvent::Exited { .. }) => {
                return Err("terminal exited before expected text".into());
            }
            Ok(LiveSessionEvent::Error(error)) => return Err(error.into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(format!("timed out waiting for {needle:?}").into())
}

fn wait_for_grid(
    session: &LiveSession,
    size: PtySize,
) -> Result<GridSnapshot, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match session.recv_timeout(Duration::from_millis(100)) {
            Ok(LiveSessionEvent::Frame(snapshot))
                if snapshot.cells.len() == usize::from(size.rows)
                    && snapshot
                        .cells
                        .iter()
                        .all(|row| row.len() == usize::from(size.cols)) =>
            {
                return Ok(snapshot);
            }
            Ok(LiveSessionEvent::Frame(_)) | Ok(LiveSessionEvent::Notification(_)) => {}
            Ok(LiveSessionEvent::Exited { .. }) => {
                return Err("terminal exited before the resized grid arrived".into());
            }
            Ok(LiveSessionEvent::Error(error)) => return Err(error.into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(format!(
        "timed out waiting for {}x{} terminal grid",
        size.cols, size.rows
    )
    .into())
}

#[test]
fn utf8_spaces_styles_cursor_and_resize_survive_the_full_stack()
-> Result<(), Box<dyn std::error::Error>> {
    let session = interactive_session()?;
    session.input("héllo terminal world\r".as_bytes().to_vec())?;
    let snapshot = wait_for_text(&session, "reply:héllo terminal world")?;

    assert!(snapshot.cursor.is_some_and(|cursor| cursor.visible));
    let has_styled_reply = snapshot
        .cells
        .iter()
        .flat_map(|row| row.iter())
        .any(|cell| !cell.text.trim().is_empty() && cell.foreground != snapshot.default_foreground);
    assert!(has_styled_reply, "reply should contain green styled cells");

    let resized = PtySize {
        rows: 9,
        cols: 52,
        pixel_width: 520,
        pixel_height: 180,
    };
    session.resize(resized, 10.0, 20.0)?;
    let resized_snapshot = wait_for_grid(&session, resized)?;
    assert_eq!(resized_snapshot.cells.len(), usize::from(resized.rows));
    assert!(
        resized_snapshot
            .cells
            .iter()
            .all(|row| row.len() == usize::from(resized.cols))
    );
    session.shutdown()?;
    Ok(())
}

#[test]
fn two_live_sessions_keep_input_and_output_isolated() -> Result<(), Box<dyn std::error::Error>> {
    let first = interactive_session()?;
    let second = interactive_session()?;
    first.input(b"first marker\r".to_vec())?;
    second.input(b"second marker\r".to_vec())?;

    let first_snapshot = wait_for_text(&first, "reply:first marker")?;
    let second_snapshot = wait_for_text(&second, "reply:second marker")?;
    assert!(!first_snapshot.text().contains("second marker"));
    assert!(!second_snapshot.text().contains("first marker"));
    first.shutdown()?;
    second.shutdown()?;
    Ok(())
}

#[test]
fn a_live_pane_applies_a_theme_without_restarting_its_process()
-> Result<(), Box<dyn std::error::Error>> {
    let session = interactive_session()?;
    let mut ansi = [Rgb {
        red: 40,
        green: 40,
        blue: 40,
    }; 16];
    ansi[2] = Rgb {
        red: 12,
        green: 210,
        blue: 120,
    };
    session.apply_theme(TerminalTheme {
        foreground: Rgb {
            red: 230,
            green: 231,
            blue: 232,
        },
        background: Rgb {
            red: 10,
            green: 11,
            blue: 12,
        },
        cursor: Rgb {
            red: 220,
            green: 221,
            blue: 222,
        },
        ansi,
    })?;
    session.input(b"theme marker\r".to_vec())?;
    let snapshot = wait_for_text(&session, "reply:theme marker")?;

    assert_eq!(snapshot.default_background.red, 10);
    assert!(
        snapshot
            .cells
            .iter()
            .flat_map(|row| row.iter())
            .any(|cell| { cell.text == "r" && cell.foreground == ansi[2] })
    );
    session.shutdown()?;
    Ok(())
}
