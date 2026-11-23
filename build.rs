use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_resource_compiler() -> Option<(PathBuf, bool)> {
    // Check PATH for rc.exe or llvm-rc.exe
    if let Some(path_var) = env::var_os("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate_rc = dir.join("rc.exe");
            if candidate_rc.exists() {
                return Some((candidate_rc, false));
            }
            let candidate_llvm_rc = dir.join("llvm-rc.exe");
            if candidate_llvm_rc.exists() {
                return Some((candidate_llvm_rc, false));
            }
        }
    }
    // Check standard LLVM installation directory
    let llvm_rc = PathBuf::from(r"C:\Program Files\LLVM\bin\llvm-rc.exe");
    if llvm_rc.exists() {
        return Some((llvm_rc, false));
    }

    // Check WinGet LLVM-MinGW directory
    let winget_llvm_rc = PathBuf::from(
        r"C:\Users\ADMIN\AppData\Local\Microsoft\WinGet\Packages\MartinStorsjo.LLVM-MinGW.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\llvm-mingw-20261117-ucrt-x86_64\bin\llvm-rc.exe",
    );
    if winget_llvm_rc.exists() {
        return Some((winget_llvm_rc, false));
    }

    // Check MinGW windres.exe
    let windres = PathBuf::from(
        r"C:\Users\ADMIN\AppData\Local\Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin\windres.exe",
    );
    if windres.exists() {
        return Some((windres, true));
    }

    None
}

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let icon_path = Path::new(&manifest_dir).join("assets").join("icon.ico");
    if !icon_path.exists() {
        eprintln!("Warning: icon file not found at: {}", icon_path.display());
        return;
    }

    let out_dir = env::var("OUT_DIR").unwrap();
    let rc_path = PathBuf::from(&out_dir).join("koko_tts.rc");
    let res_path = PathBuf::from(&out_dir).join("koko_tts.res");

    // Format .rc file: Resource ID 1 (standard main application icon in Windows)
    let icon_str = icon_path.to_str().unwrap().replace('\\', "/");
    let rc_content = format!(
        "#pragma code_page(65001)\n\
         1 ICON \"{}\"\n\
         1 VERSIONINFO\n\
         FILEVERSION 0,1,0,0\n\
         PRODUCTVERSION 0,1,0,0\n\
         FILEFLAGSMASK 0x3fL\n\
         FILEFLAGS 0x0L\n\
         FILEOS 0x40004L\n\
         FILETYPE 0x1L\n\
         FILESUBTYPE 0x0L\n\
         BEGIN\n\
           BLOCK \"StringFileInfo\"\n\
           BEGIN\n\
             BLOCK \"040904b0\"\n\
             BEGIN\n\
               VALUE \"FileDescription\", \"Kokoro-82M Text-to-Speech System Tray Reader\"\n\
               VALUE \"FileVersion\", \"0.1.0.0\"\n\
               VALUE \"InternalName\", \"koko-tts.exe\"\n\
               VALUE \"LegalCopyright\", \"Copyright (C) 2026\"\n\
               VALUE \"OriginalFilename\", \"koko-tts.exe\"\n\
               VALUE \"ProductName\", \"Kokoro TTS\"\n\
               VALUE \"ProductVersion\", \"0.1.0.0\"\n\
             END\n\
           END\n\
           BLOCK \"VarFileInfo\"\n\
           BEGIN\n\
             VALUE \"Translation\", 0x409, 1200\n\
           END\n\
         END\n",
        icon_str
    );

    if let Err(e) = fs::write(&rc_path, rc_content) {
        eprintln!("Warning: failed to write rc file: {}", e);
        return;
    }

    if let Some((compiler_path, is_windres)) = find_resource_compiler() {
        println!("Using resource compiler: {}", compiler_path.display());
        let status = if is_windres {
            Command::new(&compiler_path)
                .arg("-i")
                .arg(&rc_path)
                .arg("-o")
                .arg(&res_path)
                .arg("-O")
                .arg("coff")
                .status()
        } else {
            Command::new(&compiler_path)
                .arg(format!("/fo{}", res_path.display()))
                .arg(&rc_path)
                .status()
        };

        match status {
            Ok(s) if s.success() => {
                println!("Resource compiled successfully to {}", res_path.display());
                // Tell rustc / rust-lld to link this compiled resource file into the PE executable!
                println!("cargo:rustc-link-arg={}", res_path.display());
            }
            Ok(s) => {
                eprintln!("Warning: Resource compiler exited with status: {}", s);
            }
            Err(e) => {
                eprintln!("Warning: Failed to execute resource compiler: {}", e);
            }
        }
    } else {
        eprintln!("Warning: No Windows resource compiler (rc.exe, llvm-rc.exe, windres.exe) found!");
    }
}
