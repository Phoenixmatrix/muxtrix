use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use muxtrix::gpu;

use crate::settings::AppSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    status: Status,
    label: &'static str,
    detail: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Report {
    findings: Vec<Finding>,
}

impl Report {
    fn push(&mut self, status: Status, label: &'static str, detail: impl Into<String>) {
        self.findings.push(Finding {
            status,
            label,
            detail: detail.into(),
        });
    }

    fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.status == Status::Error)
            .count()
    }

    fn warning_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.status == Status::Warning)
            .count()
    }

    fn render(&self) -> String {
        let mut output = String::from("Muxtrix doctor\n\n");
        for finding in &self.findings {
            let marker = match finding.status {
                Status::Ok => "ok",
                Status::Warning => "warn",
                Status::Error => "error",
            };
            let _ = writeln!(output, "[{marker}] {}: {}", finding.label, finding.detail);
        }

        let errors = self.error_count();
        let warnings = self.warning_count();
        output.push('\n');
        if errors == 0 && warnings == 0 {
            output.push_str("Result: healthy\n");
        } else if errors == 0 {
            let _ = writeln!(
                output,
                "Result: healthy with {warnings} {}",
                plural(warnings, "warning", "warnings")
            );
        } else {
            let _ = writeln!(
                output,
                "Result: {errors} {}, {warnings} {}",
                plural(errors, "error", "errors"),
                plural(warnings, "warning", "warnings")
            );
        }
        output
    }
}

pub(crate) fn run(arguments: &[String]) -> Result<(), i32> {
    if !arguments.is_empty() {
        eprintln!("unknown doctor argument: {}", arguments[0]);
        eprintln!("usage: muxtrix doctor");
        return Err(2);
    }

    let report = collect();
    let output = report.render();
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
    {
        eprintln!("could not write doctor report: {error}");
        return Err(1);
    }
    if report.error_count() == 0 {
        Ok(())
    } else {
        Err(1)
    }
}

fn collect() -> Report {
    let mut report = Report::default();
    report.push(Status::Ok, "Version", env!("CARGO_PKG_VERSION"));
    report.push(
        Status::Ok,
        "Platform",
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    );
    report.push(
        Status::Ok,
        "WSL",
        if gpu::is_wsl() {
            "detected"
        } else {
            "not detected"
        },
    );

    match std::env::current_exe() {
        Ok(path) => report.push(Status::Ok, "Executable", path.display().to_string()),
        Err(error) => report.push(Status::Warning, "Executable", error.to_string()),
    }

    let settings_path = crate::settings::config_path();
    let (_, settings_error) = AppSettings::load();
    match settings_error {
        Some(error) => report.push(Status::Error, "Settings", error),
        None if settings_path.exists() => report.push(
            Status::Ok,
            "Settings",
            format!("loaded {}", settings_path.display()),
        ),
        None => report.push(
            Status::Ok,
            "Settings",
            format!(
                "using defaults; file not present at {}",
                settings_path.display()
            ),
        ),
    }

    let sessions_directory = muxtrix_sessions::sessions_directory();
    report
        .findings
        .push(session_finding(sessions_directory.as_deref()));

    report.push(
        Status::Ok,
        "Graphics environment",
        [
            gpu::WGPU_BACKEND,
            gpu::MESA_ADAPTER,
            gpu::GALLIUM_DRIVER,
            gpu::EGL_LOG_LEVEL,
        ]
        .into_iter()
        .map(|name| {
            let value = std::env::var_os(name)
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "<automatic>".into());
            format!("{name}={value}")
        })
        .collect::<Vec<_>>()
        .join(", "),
    );

    match gpu::probe_adapters() {
        Ok(probe) => {
            report.push(
                Status::Ok,
                "Graphics backends",
                format!("{:?}", probe.backends),
            );
            report.push(
                Status::Ok,
                "Available adapters",
                probe
                    .available
                    .iter()
                    .map(adapter_description)
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            let status = if gpu::adapter_is_software(&probe.selected) {
                Status::Warning
            } else {
                Status::Ok
            };
            report.push(
                status,
                "Selected adapter",
                adapter_description(&probe.selected),
            );
        }
        Err(error) => report.push(Status::Error, "Graphics adapter", error),
    }

    report
}

fn session_finding(path: Option<&Path>) -> Finding {
    match path {
        Some(path) if path.exists() && !path.is_dir() => Finding {
            status: Status::Error,
            label: "Sessions",
            detail: format!("{} exists but is not a directory", path.display()),
        },
        Some(path) if path.exists() => Finding {
            status: Status::Ok,
            label: "Sessions",
            detail: format!("directory available at {}", path.display()),
        },
        Some(path) => Finding {
            status: Status::Ok,
            label: "Sessions",
            detail: format!("directory will be created at {}", path.display()),
        },
        None => Finding {
            status: Status::Error,
            label: "Sessions",
            detail: "HOME and USERPROFILE are both unavailable".into(),
        },
    }
}

fn adapter_description(info: &wgpu::AdapterInfo) -> String {
    format!(
        "{} (backend={:?}, type={:?}, driver={}, driver_info={})",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    )
}

fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_report_is_copyable_plain_text() {
        let report = Report {
            findings: vec![Finding {
                status: Status::Ok,
                label: "Version",
                detail: "1.2.3".into(),
            }],
        };

        assert_eq!(
            report.render(),
            "Muxtrix doctor\n\n[ok] Version: 1.2.3\n\nResult: healthy\n"
        );
    }

    #[test]
    fn report_summarizes_warnings_and_errors() {
        let report = Report {
            findings: vec![
                Finding {
                    status: Status::Warning,
                    label: "Selected adapter",
                    detail: "software".into(),
                },
                Finding {
                    status: Status::Error,
                    label: "Settings",
                    detail: "invalid JSON".into(),
                },
            ],
        };

        assert!(report.render().ends_with("Result: 1 error, 1 warning\n"));
    }

    #[test]
    fn pluralizes_summary_counts() {
        assert_eq!(plural(1, "error", "errors"), "error");
        assert_eq!(plural(2, "error", "errors"), "errors");
    }

    #[test]
    fn existing_non_directory_session_path_is_an_error() {
        let root = std::env::temp_dir().join(format!(
            "muxtrix-doctor-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&root).expect("temporary directory should be created");
        let file = root.join("sessions");
        std::fs::write(&file, b"not a directory").expect("temporary file should be written");

        let finding = session_finding(Some(&file));

        assert_eq!(finding.status, Status::Error);
        assert!(finding.detail.ends_with("exists but is not a directory"));

        std::fs::remove_dir_all(root).expect("temporary directory should be removed");
    }
}
