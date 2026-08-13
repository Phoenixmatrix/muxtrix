fn main() {
    println!("cargo:rerun-if-changed=assets/muxtrix-icon.ico");

    // Build scripts run on the HOST; only the TARGET decides whether the
    // Windows resources (app icon, version strings) belong in the binary.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // Native Windows builds: winresource drives rc.exe/windres directly.
    #[cfg(windows)]
    {
        if let Err(error) = winresource::WindowsResource::new()
            .set_icon("assets/muxtrix-icon.ico")
            .set("ProductName", "Muxtrix")
            .set("FileDescription", "Muxtrix terminal workspace")
            .set("InternalName", "muxtrix")
            .set("OriginalFilename", "muxtrix.exe")
            .compile()
        {
            panic!("Muxtrix Windows resources should compile: {error}");
        }
    }

    // Cross builds from unix (cargo-zigbuild): compile the resource to a
    // COFF object and link it as a plain input — zig's cc wrapper rejects
    // the -Wl,<archive> forwarding winresource would emit.
    #[cfg(not(windows))]
    {
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR must be set");
        let manifest_dir =
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set");
        let icon = format!("{manifest_dir}/assets/muxtrix-icon.ico");
        let rc_path = format!("{out_dir}/muxtrix.rc");
        let object_path = format!("{out_dir}/muxtrix-resources.o");
        std::fs::write(&rc_path, format!("1 ICON \"{icon}\"\n"))
            .expect("resource script should be writable");
        let status = std::process::Command::new("x86_64-w64-mingw32-windres")
            .args([&rc_path, "-O", "coff", "-o", &object_path])
            .status();
        match status {
            Ok(status) if status.success() => {
                println!("cargo:rustc-link-arg-bins={object_path}");
            }
            other => panic!(
                "windres failed ({other:?}) — install binutils-mingw-w64-x86-64 \
                 and gcc-mingw-w64-x86-64 for cross-built Windows resources"
            ),
        }
    }
}
