#![cfg(unix)]

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use muxtrix_platform::{LaunchPlan, PtySession, PtySize};

#[test]
fn native_pty_accepts_input_resizes_and_streams_output() -> Result<(), Box<dyn std::error::Error>> {
    let plan = LaunchPlan {
        executable: "/bin/sh".into(),
        arguments: vec![
            "-c".into(),
            "stty -echo; IFS= read -r line; printf 'round-trip:%s\\n' \"$line\"".into(),
        ],
        working_directory: Some(PathBuf::from("/tmp")),
        environment: vec![("TERM".into(), "xterm-256color".into())],
    };
    let initial = PtySize {
        rows: 4,
        cols: 24,
        pixel_width: 240,
        pixel_height: 80,
    };
    let mut session = PtySession::spawn(&plan, initial)?;
    session.resize(PtySize {
        rows: 8,
        cols: 40,
        pixel_width: 400,
        pixel_height: 160,
    })?;
    let mut reader = session.take_reader()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = String::new();
        let result = reader.read_to_string(&mut output).map(|_| output);
        let _ = sender.send(result);
    });

    session.write_all(b"words with spaces\r")?;
    let output = receiver.recv_timeout(Duration::from_secs(2))??;

    assert!(output.contains("round-trip:words with spaces"));
    assert!(session.try_wait()?.is_some());
    Ok(())
}
