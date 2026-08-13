//! Captures what a program writes when a *real* VT is answering it.
//!
//! A bare pty answers none of the capability queries a modern TUI sends, so it
//! sees an unknown terminal and picks a fallback renderer. This probe puts the
//! same emulator muxtrix uses behind the pty — replying to queries exactly as a
//! pane does — and tees the raw bytes, so the rendering strategy being measured
//! is the one real panes get.
//!
//! Usage: cargo run -p muxtrix-terminal --example byte_probe -- <program> <rows> [seed] [out]

use std::io::Read as _;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use libghostty_vt::TerminalOptions;
use muxtrix_platform::{LaunchPlan, PtySession, PtySize};
use muxtrix_terminal::TerminalActor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = std::env::args().nth(1).unwrap_or_else(|| "claude".into());
    let rows: u16 = std::env::args()
        .nth(2)
        .and_then(|rows| rows.parse().ok())
        .unwrap_or(12);
    let seed = std::env::args().nth(3);
    let out_path = std::env::args()
        .nth(4)
        .unwrap_or_else(|| "wheel-response.bin".into());

    let mut words = program.split_whitespace().map(str::to_owned);
    let executable = words.next().unwrap_or_else(|| "claude".into());
    let size = PtySize {
        rows,
        cols: 100,
        pixel_width: 800,
        pixel_height: 600,
    };
    let mut pty = PtySession::spawn(
        &LaunchPlan {
            executable,
            arguments: words.collect(),
            working_directory: std::env::current_dir().ok(),
            environment: vec![("TERM".into(), "xterm-256color".into())],
        },
        size,
    )?;
    let mut reader = pty.take_reader()?;
    let actor = TerminalActor::spawn(TerminalOptions {
        cols: size.cols,
        rows: size.rows,
        max_scrollback: 10_000,
    })?;

    // The reader blocks, so it lives on its own thread and hands bytes back.
    let (sender, receiver) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buffer = [0_u8; 65536];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                return;
            }
        }
    });

    let pump = |pty: &mut PtySession, seconds: f32, sink: Option<&mut Vec<u8>>| {
        let mut collected = Vec::new();
        let deadline = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < deadline {
            let Ok(bytes) = receiver.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            collected.extend_from_slice(&bytes);
            let _ = actor.feed(bytes);
            // Answering the program's queries is the entire point: this is what
            // a bare pty never does.
            for response in actor.take_pty_responses().unwrap_or_default() {
                let _ = pty.write_all(&response);
            }
        }
        if let Some(sink) = sink {
            sink.extend_from_slice(&collected);
        }
        collected.len()
    };

    println!("settling…");
    pump(&mut pty, 10.0, None);
    if let Some(seed) = seed {
        println!("seeding: {seed:?}");
        pty.write_all(format!("{seed}\r").as_bytes())?;
        pump(&mut pty, 5.0, None);
    }

    // One wheel-down, SGR encoded at row 6 — exactly what a pane sends a
    // mouse-reporting program.
    println!("sending one wheel gesture…");
    pty.write_all(b"\x1b[<65;50;6M")?;
    let mut response = Vec::new();
    pump(&mut pty, 4.0, Some(&mut response));
    std::fs::write(&out_path, &response)?;
    println!("wrote {} bytes to {out_path}", response.len());

    pty.kill()?;
    Ok(())
}
