//! Headless wgpu adapter diagnostic. This binary never creates a window.

use std::process::ExitCode;

use iced::wgpu;
use muxtrix::gpu;

#[derive(Debug, Default)]
struct Requirements {
    hardware: bool,
    adapter_substring: Option<String>,
}

fn main() -> ExitCode {
    if let Err(error) = gpu::ensure_wsl_gpu_defaults() {
        eprintln!("failed to apply process-local WSL GPU defaults: {error}");
        return ExitCode::FAILURE;
    }

    let requirements = match parse_requirements(std::env::args().skip(1)) {
        Ok(requirements) => requirements,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: muxtrix-gpu-probe [--require-hardware] [--require-adapter SUBSTRING]"
            );
            return ExitCode::FAILURE;
        }
    };

    match probe(&requirements) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("GPU probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_requirements(mut arguments: impl Iterator<Item = String>) -> Result<Requirements, String> {
    let mut requirements = Requirements::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--require-hardware" => requirements.hardware = true,
            "--require-adapter" => {
                requirements.adapter_substring = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--require-adapter needs a substring".to_owned())?,
                );
            }
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    Ok(requirements)
}

fn probe(requirements: &Requirements) -> Result<(), String> {
    let backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all());
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::default()
    });

    println!("WSL detected: {}", gpu::is_wsl());
    println!(
        "{}={}",
        gpu::WGPU_BACKEND,
        std::env::var(gpu::WGPU_BACKEND).unwrap_or_else(|_| "<automatic>".into())
    );
    println!(
        "{}={}",
        gpu::MESA_ADAPTER,
        std::env::var(gpu::MESA_ADAPTER).unwrap_or_else(|_| "<automatic>".into())
    );
    println!(
        "{}={}",
        gpu::GALLIUM_DRIVER,
        std::env::var(gpu::GALLIUM_DRIVER).unwrap_or_else(|_| "<automatic>".into())
    );
    println!(
        "{}={}",
        gpu::EGL_LOG_LEVEL,
        std::env::var(gpu::EGL_LOG_LEVEL).unwrap_or_else(|_| "<automatic>".into())
    );

    let available = instance.enumerate_adapters(backends);
    if available.is_empty() {
        return Err("wgpu did not enumerate any adapters".into());
    }
    println!("Available adapters:");
    for adapter in &available {
        print_adapter("  -", &adapter.get_info());
    }

    let selected =
        iced::futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|error| error.to_string())?;
    let info = selected.get_info();
    println!("Selected adapter:");
    print_adapter("  ", &info);

    if requirements.hardware && info.device_type == wgpu::DeviceType::Cpu {
        return Err(format!("selected CPU adapter `{}`", info.name));
    }
    if requirements.hardware && info.name.to_ascii_lowercase().contains("llvmpipe") {
        return Err(format!("selected software adapter `{}`", info.name));
    }
    if let Some(required) = &requirements.adapter_substring {
        let required = required.to_ascii_lowercase();
        let description =
            format!("{} {} {}", info.name, info.driver, info.driver_info).to_ascii_lowercase();
        if !description.contains(&required) {
            return Err(format!(
                "selected adapter `{}` does not contain required substring `{required}`",
                info.name
            ));
        }
    }

    println!("GPU requirements satisfied.");
    Ok(())
}

fn print_adapter(prefix: &str, info: &wgpu::AdapterInfo) {
    println!(
        "{prefix} name={:?} backend={:?} type={:?} driver={:?} driver_info={:?}",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probe_requirements() {
        let parsed = parse_requirements(
            ["--require-hardware", "--require-adapter", "NVIDIA"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("arguments should parse");
        assert!(parsed.hardware);
        assert_eq!(parsed.adapter_substring.as_deref(), Some("NVIDIA"));
    }
}
