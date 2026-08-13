#![cfg(target_os = "linux")]

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConfigureWindowAux, ConnectionExt as _, InputFocus,
    KEY_PRESS_EVENT, KEY_RELEASE_EVENT, MOTION_NOTIFY_EVENT, MapState,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

struct ProcessGuard(Option<Child>);

impl ProcessGuard {
    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("guard should own a child")
    }

    fn take(&mut self) -> Child {
        self.0.take().expect("guard should own a child")
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn real_app_runs_terminal_workspace_flow_on_private_x_server()
-> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("MUXTRIX_E2E_DISPLAY_READY").is_none() {
        // The private screen has to hold the requested window: pointer input is
        // delivered in root coordinates, so a window wider than the screen puts
        // the scrollbar and pane edges outside anything XTEST can click.
        let (screen_width, screen_height) = requested_viewport()?;
        let screen = format!(
            "-screen 0 {}x{}x24",
            screen_width.max(1_280),
            screen_height.max(800)
        );
        let output = Command::new("xvfb-run")
            .args([
                "-a",
                "-s",
                &screen,
                std::env::current_exe()?
                    .to_str()
                    .ok_or("non-UTF-8 test path")?,
                "--exact",
                "real_app_runs_terminal_workspace_flow_on_private_x_server",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("MUXTRIX_E2E_DISPLAY_READY", "1")
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "nested Xvfb E2E test failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        return Ok(());
    }

    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let display = std::env::var("DISPLAY")?;
    eprintln!("headless E2E display ready at {display}");
    let (connection, screen_number) = connect_with_retry(&display, Duration::from_secs(5))?;
    let root = connection.setup().roots[screen_number].root;

    let report_path =
        std::env::temp_dir().join(format!("muxtrix-e2e-{}-{unique}.json", std::process::id()));
    let config_path = std::env::temp_dir().join(format!(
        "muxtrix-e2e-settings-{}-{unique}.json",
        std::process::id()
    ));
    let control_path = std::env::temp_dir().join(format!(
        "muxtrix-e2e-control-{}-{unique}.sock",
        std::process::id()
    ));
    let home_path =
        std::env::temp_dir().join(format!("muxtrix-e2e-home-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&home_path)?;
    let requested_screenshot = std::env::var_os("MUXTRIX_E2E_SCREENSHOT_RGBA");
    let screenshot_path = requested_screenshot.clone().map_or_else(
        || {
            std::env::temp_dir().join(format!(
                "muxtrix-e2e-screenshot-{}-{unique}.rgba",
                std::process::id()
            ))
        },
        std::path::PathBuf::from,
    );
    // A capture can be pinned to a specific settings profile by pointing this
    // at a settings file. Without it the app starts on defaults, which cannot
    // reproduce a rendering bug that depends on the configured font or weight.
    if let Some(profile) = std::env::var_os("MUXTRIX_E2E_SETTINGS") {
        std::fs::copy(&profile, &config_path)?;
        eprintln!("seeded settings profile from {}", profile.to_string_lossy());
    }
    let mut app = Command::new(env!("CARGO_BIN_EXE_muxtrix"))
        .env("DISPLAY", &display)
        .env_remove("WAYLAND_DISPLAY")
        .env("WINIT_UNIX_BACKEND", "x11")
        .env("XDG_SESSION_TYPE", "x11")
        .env("MUXTRIX_E2E_REPORT", &report_path)
        .env("MUXTRIX_CONFIG_PATH", &config_path)
        .env("MUXTRIX_CONTROL_ENDPOINT", &control_path)
        .env("MUXTRIX_E2E_SCREENSHOT_RGBA", &screenshot_path)
        .env("HOME", &home_path)
        .env("SHELL", "/bin/sh")
        .env("WGPU_BACKEND", "vulkan")
        .env("GALLIUM_DRIVER", "llvmpipe")
        .env("MESA_D3D12_DEFAULT_ADAPTER_NAME", "none")
        .env("EGL_LOG_LEVEL", "fatal")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map(|child| ProcessGuard(Some(child)))?;

    let window = wait_for_app_window(&connection, root, Duration::from_secs(8))?;
    let final_viewport = requested_viewport()?;
    let window_origin = connection
        .translate_coordinates(window, root, 0, 0)?
        .reply()?;
    let terminal_x = window_origin.dst_x.saturating_add(
        i16::try_from(final_viewport.0 / 2).map_err(|_| "viewport width exceeds X11 range")?,
    );
    let terminal_y = window_origin.dst_y.saturating_add(
        i16::try_from(final_viewport.1 / 2).map_err(|_| "viewport height exceeds X11 range")?,
    );
    eprintln!("Muxtrix window mapped as X11 window {window}");
    let control_response = control_request(&control_path, r#"{"method":"ping"}"#)?;
    assert_eq!(control_response["ok"], true);
    assert_eq!(control_response["message"], "pong");
    eprintln!("verified the live local control service");
    stress_resize_window(&connection, window, &mut app, final_viewport)?;
    let control_response = control_request(&control_path, r#"{"method":"ping"}"#)?;
    assert_eq!(control_response["ok"], true);
    eprintln!("survived repeated real-window resizes with control service responsive");
    connection
        .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?
        .check()?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(250));
    chord(&connection, 0xffe3, 'p' as u32)?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(200));
    tap_keysym(&connection, 0xff54)?;
    tap_keysym(&connection, 0xff54)?;
    tap_keysym(&connection, 0xff52)?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(300));
    tap_keysym(&connection, 0xff1b)?;
    chord(&connection, 0xffe3, ',' as u32)?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(500));
    tap_keysym(&connection, 0xff1b)?;
    connection.flush()?;
    eprintln!("exercised command palette and settings shortcuts");
    connection
        .xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, terminal_x, terminal_y, 0)?
        .check()?;
    type_text(&connection, "echo pane-menu-click-away-ready")?;
    tap_keysym(&connection, 0xff0d)?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(300));
    click_at(&connection, root, terminal_x, terminal_y)?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(200));
    eprintln!("opened the pane menu and dismissed it with an outside click");
    connection
        .xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, terminal_x, terminal_y, 0)?
        .check()?;
    type_text(&connection, "seq 1 120")?;
    tap_keysym(&connection, 0xff0d)?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(350));
    // The content area carries 8px padding and each pane card a 1px border,
    // so the right pane's scrollbar hit area ends ~9px inside the window edge.
    let scrollbar_x = window_origin.dst_x.saturating_add(
        i16::try_from(final_viewport.0.saturating_sub(14))
            .map_err(|_| "viewport width exceeds X11 range")?,
    );
    let scrollbar_y = terminal_y;
    let scrollbar_drag_y = scrollbar_y.saturating_sub(120);
    connection
        .xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, scrollbar_x, scrollbar_y, 0)?
        .check()?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(100));
    connection
        .xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, scrollbar_x, scrollbar_y, 0)?
        .check()?;
    connection
        .xtest_fake_input(
            MOTION_NOTIFY_EVENT,
            0,
            0,
            root,
            scrollbar_x,
            scrollbar_drag_y,
            0,
        )?
        .check()?;
    connection
        .xtest_fake_input(
            BUTTON_RELEASE_EVENT,
            1,
            0,
            root,
            scrollbar_x,
            scrollbar_drag_y,
            0,
        )?
        .check()?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(150));
    let scrollbar_bottom_y = window_origin.dst_y.saturating_add(
        i16::try_from(final_viewport.1.saturating_sub(20))
            .map_err(|_| "viewport height exceeds X11 range")?,
    );
    connection
        .xtest_fake_input(
            MOTION_NOTIFY_EVENT,
            0,
            0,
            root,
            scrollbar_x,
            scrollbar_bottom_y,
            0,
        )?
        .check()?;
    connection
        .xtest_fake_input(
            BUTTON_PRESS_EVENT,
            1,
            0,
            root,
            scrollbar_x,
            scrollbar_bottom_y,
            0,
        )?
        .check()?;
    connection
        .xtest_fake_input(
            BUTTON_RELEASE_EVENT,
            1,
            0,
            root,
            scrollbar_x,
            scrollbar_bottom_y,
            0,
        )?
        .check()?;
    connection.flush()?;
    eprintln!("clicked and dragged the terminal scrollbar");
    connection
        .xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, terminal_x, terminal_y, 0)?
        .check()?;
    connection.flush()?;
    thread::sleep(Duration::from_millis(75));
    for _ in 0..50 {
        connection
            .xtest_fake_input(BUTTON_PRESS_EVENT, 5, 0, root, terminal_x, terminal_y, 0)?
            .check()?;
        connection
            .xtest_fake_input(BUTTON_RELEASE_EVENT, 5, 0, root, terminal_x, terminal_y, 0)?
            .check()?;
    }
    connection.flush()?;
    eprintln!("injected terminal mouse-wheel scroll");
    thread::sleep(Duration::from_millis(150));
    // Geometric symbols whose ink fills the whole advance box, repeated across
    // consecutive columns so every sub-pixel phase of the fractional cell width
    // is covered. U+23F5, U+2733 and U+276F are what Claude Code draws in its
    // footer and agent list, and are the glyphs reported as clipped. Six
    // adjacent glyphs span more than the five-column period of the fractional
    // advance, so every phase appears. Octal escapes keep the keystrokes
    // unshifted, which is all the harness keymap can tap.
    let glyph_line = format!(
        "printf '{}{}{}\\n'",
        "\\342\\217\\265".repeat(6),
        "\\342\\234\\263".repeat(6),
        "\\342\\235\\257".repeat(6)
    );
    type_text(&connection, &glyph_line)?;
    tap_keysym(&connection, 0xff0d)?;
    connection.flush()?;
    eprintln!("injected glyph clipping repro line");
    thread::sleep(Duration::from_millis(250));
    type_text(
        &connection,
        "printf 'alpha beta https\\072//example.com/docs\\n'",
    )?;
    tap_keysym(&connection, 0xff0d)?;
    connection.flush()?;
    eprintln!("injected terminal command with spaces");

    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if app.child_mut().try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = app.child_mut().kill();
            return Err("Muxtrix E2E app did not exit before its deadline".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    let output = app.take().wait_with_output()?;
    eprintln!("Muxtrix exited with {}", output.status);
    if !output.status.success() {
        return Err(format!(
            "Muxtrix E2E process failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(&report_path)?)?;
    let _ = std::fs::remove_file(&report_path);
    let _ = std::fs::remove_file(&config_path);
    let _ = std::fs::remove_file(&control_path);
    let _ = std::fs::remove_dir_all(&home_path);
    if requested_screenshot.is_none() {
        let _ = std::fs::remove_file(&screenshot_path);
    }
    assert_eq!(report["success"], true, "E2E report: {report:#}");
    for check in [
        "real_window_and_wgpu_frame",
        "command_palette_shortcut_and_render",
        "command_palette_keyboard_navigation",
        "settings_shortcut_and_render",
        "external_keyboard_input_with_spaces",
        "terminal_url_decoration_rendered",
        "focused_cursor_visible",
        "horizontal_split_resized",
        "vertical_split_resized",
        "split_pane_grids_match_their_panes",
        "pointer_drag_selects_terminal_text",
        "independent_terminal_sessions",
        "focused_pane_close_cleanup",
        "terminal_exit_detach_and_restart",
        "osc_agent_notification_fleet_attention_and_clear",
        "fleet_collapse_pane_maximize_and_overflow",
        "pane_overflow_click_away",
        "terminal_mouse_wheel_scrollback",
        "terminal_scrollbar_click_and_drag",
        "terminal_drawing_glyphs_are_pixel_continuous",
        "terminal_rounded_box_is_pixel_connected",
        "terminal_heavy_box_is_pixel_connected",
    ] {
        assert_eq!(report["checks"][check], true, "missing E2E check {check}");
    }
    assert_eq!(
        report["metrics"]["screenshot_width"],
        u64::from(final_viewport.0)
    );
    assert_eq!(
        report["metrics"]["screenshot_height"],
        u64::from(final_viewport.1)
    );
    Ok(())
}

fn stress_resize_window(
    connection: &RustConnection,
    window: u32,
    app: &mut ProcessGuard,
    final_viewport: (u32, u32),
) -> Result<(), Box<dyn std::error::Error>> {
    let mut viewports = vec![
        (1_120, 720),
        (960, 640),
        (820, 560),
        (1_024, 680),
        (900, 600),
    ];
    viewports.push(final_viewport);
    for (width, height) in viewports {
        connection
            .configure_window(
                window,
                &ConfigureWindowAux::new().width(width).height(height),
            )?
            .check()?;
        connection.flush()?;
        thread::sleep(Duration::from_millis(45));
        if let Some(status) = app.child_mut().try_wait()? {
            return Err(
                format!("Muxtrix exited during a {width}x{height} resize: {status}").into(),
            );
        }
    }
    Ok(())
}

fn requested_viewport() -> Result<(u32, u32), Box<dyn std::error::Error>> {
    let Some(value) = std::env::var_os("MUXTRIX_E2E_VIEWPORT") else {
        return Ok((1_280, 800));
    };
    let value = value.to_string_lossy();
    let (width, height) = value
        .split_once('x')
        .ok_or("MUXTRIX_E2E_VIEWPORT must use WIDTHxHEIGHT")?;
    let width = width.parse::<u32>()?;
    let height = height.parse::<u32>()?;
    if width < 720 || height < 480 {
        return Err("MUXTRIX_E2E_VIEWPORT is below the supported 720x480 minimum".into());
    }
    Ok((width, height))
}

fn control_request(
    path: &std::path::Path,
    request: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use std::os::unix::net::UnixStream;

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut stream = loop {
        match UnixStream::connect(path) {
            Ok(stream) => break stream,
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error.into()),
        }
    };
    writeln!(stream, "{request}")?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    Ok(serde_json::from_str(&response)?)
}

fn connect_with_retry(
    display: &str,
    timeout: Duration,
) -> Result<(RustConnection, usize), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        match x11rb::connect(Some(display)) {
            Ok(connection) => return Ok(connection),
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn wait_for_app_window(
    connection: &RustConnection,
    root: u32,
    timeout: Duration,
) -> Result<u32, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        for window in connection.query_tree(root)?.reply()?.children {
            let geometry = connection.get_geometry(window)?.reply()?;
            let attributes = connection.get_window_attributes(window)?.reply()?;
            if geometry.width >= 1_200
                && geometry.height >= 700
                && attributes.map_state == MapState::VIEWABLE
            {
                return Ok(window);
            }
        }
        if Instant::now() >= deadline {
            return Err("Muxtrix window did not appear on the private X server".into());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn type_text(connection: &RustConnection, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    for character in text.chars() {
        tap_keysym(connection, character as u32)?;
        // XTEST delivers faster than the shell drains its input, and a long
        // string loses characters partway through without this pause.
        thread::sleep(Duration::from_millis(4));
    }
    Ok(())
}

fn click_at(
    connection: &RustConnection,
    root: u32,
    x: i16,
    y: i16,
) -> Result<(), Box<dyn std::error::Error>> {
    connection
        .xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x, y, 0)?
        .check()?;
    connection
        .xtest_fake_input(BUTTON_PRESS_EVENT, 1, 0, root, x, y, 0)?
        .check()?;
    connection
        .xtest_fake_input(BUTTON_RELEASE_EVENT, 1, 0, root, x, y, 0)?
        .check()?;
    Ok(())
}

fn tap_keysym(connection: &RustConnection, keysym: u32) -> Result<(), Box<dyn std::error::Error>> {
    let keycode = keycode_for_keysym(connection, keysym)?;
    connection
        .xtest_fake_input(KEY_PRESS_EVENT, keycode, 0, 0, 0, 0, 0)?
        .check()?;
    connection
        .xtest_fake_input(KEY_RELEASE_EVENT, keycode, 0, 0, 0, 0, 0)?
        .check()?;
    Ok(())
}

fn chord(
    connection: &RustConnection,
    modifier_keysym: u32,
    key_keysym: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let modifier = keycode_for_keysym(connection, modifier_keysym)?;
    let key = keycode_for_keysym(connection, key_keysym)?;
    connection
        .xtest_fake_input(KEY_PRESS_EVENT, modifier, 0, 0, 0, 0, 0)?
        .check()?;
    connection
        .xtest_fake_input(KEY_PRESS_EVENT, key, 0, 0, 0, 0, 0)?
        .check()?;
    connection
        .xtest_fake_input(KEY_RELEASE_EVENT, key, 0, 0, 0, 0, 0)?
        .check()?;
    connection
        .xtest_fake_input(KEY_RELEASE_EVENT, modifier, 0, 0, 0, 0, 0)?
        .check()?;
    Ok(())
}

fn keycode_for_keysym(
    connection: &RustConnection,
    keysym: u32,
) -> Result<u8, Box<dyn std::error::Error>> {
    let setup = connection.setup();
    let count = setup.max_keycode - setup.min_keycode + 1;
    let mapping = connection
        .get_keyboard_mapping(setup.min_keycode, count)?
        .reply()?;
    let width = usize::from(mapping.keysyms_per_keycode);
    for (offset, keysyms) in mapping.keysyms.chunks(width).enumerate() {
        if keysyms.first().copied() == Some(keysym) {
            return Ok(setup.min_keycode + u8::try_from(offset)?);
        }
    }
    Err(format!("Xvfb keymap has no unshifted keysym 0x{keysym:x}").into())
}
