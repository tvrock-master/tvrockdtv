use std::{path::Path, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())?
}

fn main() {
    for source in [
        "native/build.ps1",
        "native/CMakeLists.txt",
        "native/dependencies.json",
        "native/srt.cpp",
        "native/tools/Dependencies.ps1",
        "native/tools/Processes.ps1",
        "native/tools/ExpandZip.ps1",
        "native/cmake/dependencies/CMakeLists.txt",
        "native/cmake/DetectMsvcIncludes.cmake",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out = std::env::var("OUT_DIR").unwrap();
        let arch = if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86") {
            "x86"
        } else {
            "x64"
        };
        // The native script supports Windows PowerShell 5.1, which ships with
        // Windows. Do not require a separate PowerShell 7 installation.
        let status = Command::new("powershell.exe")
            // Let Windows PowerShell initialize its own module search paths.
            // Inheriting PowerShell 7's PSModulePath through Cargo can prevent
            // built-in commands such as Get-FileHash from loading on CI.
            .env_remove("PSModulePath")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "native/build.ps1",
                "-Architecture",
                arch,
                "-OutputDir",
                &out,
            ])
            .status()
            .expect("Cannot start Windows PowerShell (powershell.exe) for the native SRT build");
        assert!(status.success(), "Static SRT build failed");
        println!("cargo:rustc-link-search=native={out}/lib");
        for lib in ["tvrock_srt", "srt_static", "mbedcrypto"] {
            println!("cargo:rustc-link-lib=static={lib}");
        }
        println!("cargo:rustc-link-lib=ws2_32");
        println!("cargo:rustc-link-lib=bcrypt");
    }

    println!("cargo:rerun-if-changed=build.rs");
    // Track source edits and Git changes, including tags and linked worktrees.
    if let Some(files) = git(&[
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ]) {
        for file in files.split('\0').filter(|file| !file.is_empty()) {
            println!("cargo:rerun-if-changed={file}");
        }
    }
    for name in ["HEAD", "index", "refs", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            let path = path.trim();
            if Path::new(path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }

    let version = git(&["describe", "--tags", "--exact-match", "HEAD"])
        .filter(|_| git(&["status", "--porcelain"]).is_some_and(|s| s.is_empty()))
        .map(|tag| tag.trim().to_owned())
        .filter(|tag| !tag.is_empty())
        .unwrap_or_else(|| "dev".into());
    println!("cargo:rustc-env=TVROCKDTV_VERSION={version}");
}
