//! Measures how a real program answers a wheel gesture inside this crate's
//! own terminal stack — a VT that answers capability queries, unlike a bare
//! pty. Reports whether the content shifted, and whether that shift was a
//! terminal scroll (which a selection can follow) or a repaint (which it
//! cannot).
//!
//! Usage: cargo run -p muxtrix-terminal --example scroll_probe -- <program>

use std::time::{Duration, Instant};

use libghostty_vt::TerminalOptions;
use muxtrix_platform::{LaunchPlan, PtySize};
use muxtrix_terminal::{LiveSession, LiveSessionEvent};

fn drain(session: &LiveSession, seconds: f32) {
    let deadline = Instant::now() + Duration::from_secs_f32(seconds);
    while Instant::now() < deadline {
        if let Ok(LiveSessionEvent::Exited { .. }) =
            session.recv_timeout(Duration::from_millis(100))
        {
            return;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = std::env::args().nth(1).unwrap_or_else(|| "claude".into());
    let rows: u16 = std::env::args()
        .nth(2)
        .and_then(|rows| rows.parse().ok())
        .unwrap_or(30);
    let seed = std::env::args().nth(3);
    // Keep the scroll well short of a screen: the shift check needs enough
    // surviving rows to recognize movement.
    let scroll: isize = std::env::args()
        .nth(4)
        .and_then(|lines| lines.parse().ok())
        .unwrap_or(9);
    let size = PtySize {
        rows,
        cols: 100,
        pixel_width: 800,
        pixel_height: 600,
    };
    let mut words = program.split_whitespace().map(str::to_owned);
    let executable = words.next().unwrap_or_else(|| "claude".into());
    let session = LiveSession::spawn(
        LaunchPlan {
            executable,
            arguments: words.collect(),
            working_directory: std::env::current_dir().ok(),
            environment: std::env::var("PROBE_ENV").ok().map_or_else(
                || vec![("TERM".to_owned(), "xterm-256color".to_owned())],
                |extra| {
                    let mut environment = vec![("TERM".to_owned(), "xterm-256color".to_owned())];
                    for pair in extra.split(',') {
                        if let Some((name, value)) = pair.split_once('=') {
                            environment.push((name.to_owned(), value.to_owned()));
                        }
                    }
                    environment
                },
            ),
        },
        size,
        TerminalOptions {
            cols: size.cols,
            rows: size.rows,
            max_scrollback: 10_000,
        },
    )?;

    println!("waiting for {program} to settle…");
    drain(&session, 10.0);
    if let Some(seed) = seed {
        println!("seeding: {seed:?}");
        session.input(format!("{seed}\r").into_bytes())?;
        drain(&session, 5.0);
    }
    let before = session.snapshot()?;
    println!(
        "before: scrollbar total={} offset={} visible={}",
        before.scrollbar.total, before.scrollbar.offset, before.scrollbar.visible
    );

    for (index, row) in before.rows.iter().enumerate() {
        println!("  before[{index:2}] {row}");
    }

    session.wheel(scroll, Some((50, 6)))?;
    std::thread::sleep(Duration::from_millis(250));
    drain(&session, 3.0);
    let after = session.snapshot()?;
    println!(
        "after:  scrollbar total={} offset={} visible={}",
        after.scrollbar.total, after.scrollbar.offset, after.scrollbar.visible
    );

    // Did the same text end up on different rows? That is the shift a
    // selection would have to follow.
    let mut shift = None;
    for candidate in 1..before.rows.len() {
        let matches = before
            .rows
            .iter()
            .skip(candidate)
            .zip(after.rows.iter())
            .filter(|(old, new)| !old.trim().is_empty() && old == new)
            .count();
        if matches >= 5 {
            shift = Some(candidate);
            break;
        }
    }
    let identical = before
        .rows
        .iter()
        .zip(after.rows.iter())
        .filter(|(old, new)| old == new)
        .count();
    println!(
        "rows identical in place: {identical}/{}, content shifted by: {shift:?}",
        before.rows.len()
    );
    println!(
        "viewport moved: {}",
        after.scrollbar.offset != before.scrollbar.offset
    );

    // Could a selection follow its text? Only if the text it covered is still
    // somewhere on the repainted screen.
    let mut findable = 0;
    let mut moved = 0;
    let mut vanished = Vec::new();
    for (index, row) in before.rows.iter().enumerate() {
        if row.trim().is_empty() {
            continue;
        }
        match after.rows.iter().position(|candidate| candidate == row) {
            Some(new_index) => {
                findable += 1;
                if new_index != index {
                    moved += 1;
                }
            }
            None => vanished.push(index),
        }
    }
    println!("\nselection-follow check across the repaint:");
    println!("  rows still findable somewhere: {findable} (of which {moved} moved rows)");
    println!("  rows no longer on screen: {vanished:?}");

    session.shutdown()?;
    Ok(())
}
