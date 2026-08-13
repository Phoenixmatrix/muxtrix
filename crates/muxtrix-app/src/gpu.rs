//! Process-local graphics defaults for WSLg.

use std::ffi::OsString;
use std::io;

pub const WGPU_BACKEND: &str = "WGPU_BACKEND";
pub const MESA_ADAPTER: &str = "MESA_D3D12_DEFAULT_ADAPTER_NAME";
pub const GALLIUM_DRIVER: &str = "GALLIUM_DRIVER";
pub const EGL_LOG_LEVEL: &str = "EGL_LOG_LEVEL";
const BOOTSTRAPPED: &str = "MUXTRIX_WSL_GPU_BOOTSTRAPPED";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentDefault {
    pub name: &'static str,
    pub value: &'static str,
}

#[derive(Debug)]
struct WslStartupEnvironment {
    is_wsl: bool,
    backend: Option<OsString>,
    adapter: Option<OsString>,
    gallium_driver: Option<OsString>,
    egl_log_level: Option<OsString>,
    nvidia_wsl_driver_is_present: bool,
    already_bootstrapped: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct ReexecEnvironmentPlan {
    set: Vec<EnvironmentDefault>,
    remove: Vec<&'static str>,
}

#[must_use]
pub fn planned_wsl_defaults(
    is_wsl: bool,
    wgpu_backend_is_set: bool,
    mesa_adapter_is_set: bool,
    gallium_driver_is_set: bool,
    egl_log_level_is_set: bool,
    nvidia_wsl_driver_is_present: bool,
) -> Vec<EnvironmentDefault> {
    if !is_wsl {
        return Vec::new();
    }

    let mut defaults = Vec::new();
    if !wgpu_backend_is_set {
        defaults.push(EnvironmentDefault {
            name: WGPU_BACKEND,
            value: "gl",
        });
    }
    if !mesa_adapter_is_set && nvidia_wsl_driver_is_present {
        defaults.push(EnvironmentDefault {
            name: MESA_ADAPTER,
            value: "NVIDIA",
        });
    }
    if !gallium_driver_is_set {
        defaults.push(EnvironmentDefault {
            name: GALLIUM_DRIVER,
            value: "d3d12",
        });
    }
    if !egl_log_level_is_set {
        defaults.push(EnvironmentDefault {
            name: EGL_LOG_LEVEL,
            value: "fatal",
        });
    }
    defaults
}

#[must_use]
pub fn is_wsl() -> bool {
    if std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some() {
        return true;
    }

    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
}

/// Replaces the current Linux process with one carrying WSL GPU defaults.
///
/// Rust 2024 makes mutating the current process environment unsafe in a
/// multithreaded program. Re-executing before Iced starts is safe, keeps the
/// defaults process-local, and leaves explicit user values untouched.
pub fn ensure_wsl_gpu_defaults() -> io::Result<()> {
    ensure_wsl_gpu_defaults_with(WslStartupEnvironment {
        is_wsl: is_wsl(),
        backend: std::env::var_os(WGPU_BACKEND),
        adapter: std::env::var_os(MESA_ADAPTER),
        gallium_driver: std::env::var_os(GALLIUM_DRIVER),
        egl_log_level: std::env::var_os(EGL_LOG_LEVEL),
        nvidia_wsl_driver_is_present: std::path::Path::new("/usr/lib/wsl/lib/nvidia-smi").is_file(),
        already_bootstrapped: std::env::var_os(BOOTSTRAPPED).is_some(),
    })
}

fn ensure_wsl_gpu_defaults_with(environment: WslStartupEnvironment) -> io::Result<()> {
    let Some(_plan) = planned_reexec_environment(&environment) else {
        return Ok(());
    };

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;

        let executable = std::env::current_exe()?;
        let mut command = std::process::Command::new(executable);
        command.args(std::env::args_os().skip(1));
        for default in _plan.set {
            command.env(default.name, default.value);
        }
        for name in _plan.remove {
            command.env_remove(name);
        }
        Err(command.exec())
    }

    #[cfg(not(target_os = "linux"))]
    Ok(())
}

fn planned_reexec_environment(
    environment: &WslStartupEnvironment,
) -> Option<ReexecEnvironmentPlan> {
    if environment.already_bootstrapped {
        return None;
    }

    let mut set = planned_wsl_defaults(
        environment.is_wsl,
        environment.backend.is_some(),
        environment.adapter.is_some(),
        environment.gallium_driver.is_some(),
        environment.egl_log_level.is_some(),
        environment.nvidia_wsl_driver_is_present,
    );
    if set.is_empty() {
        return None;
    }
    set.push(EnvironmentDefault {
        name: BOOTSTRAPPED,
        value: "1",
    });

    Some(ReexecEnvironmentPlan {
        set,
        remove: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_defaults_prefer_mesa_d3d12_on_nvidia() {
        assert_eq!(
            planned_wsl_defaults(true, false, false, false, false, true),
            vec![
                EnvironmentDefault {
                    name: WGPU_BACKEND,
                    value: "gl",
                },
                EnvironmentDefault {
                    name: MESA_ADAPTER,
                    value: "NVIDIA",
                },
                EnvironmentDefault {
                    name: GALLIUM_DRIVER,
                    value: "d3d12",
                },
                EnvironmentDefault {
                    name: EGL_LOG_LEVEL,
                    value: "fatal",
                },
            ]
        );
    }

    #[test]
    fn explicit_user_values_are_preserved() {
        assert!(planned_wsl_defaults(true, true, true, true, true, true).is_empty());
        assert_eq!(
            planned_wsl_defaults(true, true, false, true, true, true),
            vec![EnvironmentDefault {
                name: MESA_ADAPTER,
                value: "NVIDIA",
            }]
        );
    }

    #[test]
    fn non_wsl_linux_gets_no_defaults() {
        assert!(planned_wsl_defaults(false, false, false, false, false, true).is_empty());
    }

    #[test]
    fn non_nvidia_wsl_keeps_automatic_adapter_selection() {
        let defaults = planned_wsl_defaults(true, false, false, false, false, false);
        assert!(defaults.iter().all(|default| default.name != MESA_ADAPTER));
    }

    #[test]
    fn explicit_egl_log_level_is_preserved() {
        let defaults = planned_wsl_defaults(true, true, true, true, true, true);
        assert!(defaults.iter().all(|default| default.name != EGL_LOG_LEVEL));
    }

    #[test]
    fn wsl_defaults_never_override_the_native_window_system() {
        let plan = planned_reexec_environment(&WslStartupEnvironment {
            is_wsl: true,
            backend: None,
            adapter: None,
            gallium_driver: None,
            egl_log_level: None,
            nvidia_wsl_driver_is_present: true,
            already_bootstrapped: false,
        })
        .expect("missing WSL defaults should require re-execution");

        assert!(plan.remove.is_empty());
        assert!(plan.set.iter().all(|variable| !matches!(
            variable.name,
            "WAYLAND_DISPLAY" | "DISPLAY" | "XDG_SESSION_TYPE"
        )));
    }
}
