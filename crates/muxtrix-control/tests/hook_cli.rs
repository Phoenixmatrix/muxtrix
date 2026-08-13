use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn cli_add_remove_and_readd_are_clean_and_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root =
        std::env::temp_dir().join(format!("muxtrix-hook-cli-{}-{unique}", std::process::id()));
    let home = root.join("home");
    let project = root.join("project");
    let state = root.join("state");
    let bridge_command = "/mnt/c/Program Files/Muxtrix/muxtrixctl.exe";
    std::fs::create_dir_all(&project)?;

    let run = |action: &str| -> Result<std::process::Output, std::io::Error> {
        Command::new(env!("CARGO_BIN_EXE_muxtrixctl"))
            .args(["hooks", action, "all", "--scope", "user", "--project"])
            .arg(&project)
            .args(["--hook-command", bridge_command])
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_STATE_HOME", &state)
            .env("LOCALAPPDATA", &state)
            .output()
    };

    let added = run("add")?;
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let codex = home.join(".codex/hooks.json");
    let claude = home.join(".claude/settings.json");
    assert_eq!(managed_markers(&codex)?, 8);
    assert_eq!(managed_markers(&claude)?, 9);
    assert!(std::fs::read_to_string(&codex)?.contains(bridge_command));
    assert!(std::fs::read_to_string(&claude)?.contains(bridge_command));

    let duplicate = run("add")?;
    assert!(duplicate.status.success());
    assert_eq!(managed_markers(&codex)?, 8);
    assert_eq!(managed_markers(&claude)?, 9);

    let removed = run("remove")?;
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!codex.exists());
    assert!(!claude.exists());

    let readded = run("re-add")?;
    assert!(readded.status.success());
    assert_eq!(managed_markers(&codex)?, 8);
    assert_eq!(managed_markers(&claude)?, 9);
    assert!(run("remove")?.status.success());
    assert!(!codex.exists());
    assert!(!claude.exists());

    let _ = std::fs::remove_dir_all(root);
    Ok(())
}

fn managed_markers(path: &std::path::Path) -> Result<usize, Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string(path)?;
    Ok(contents.matches("muxtrix-hook-v1").count())
}
