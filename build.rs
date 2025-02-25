use std::i64;
use std::process::Command;
use std::str;
use which::which;
#[cfg(windows)]
use windows::core::{HSTRING, PCSTR, PCWSTR, PSTR, PWSTR};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

#[cfg(windows)]
trait IntoPWSTR {
    fn into_pwstr(self) -> PWSTR;
}

#[cfg(windows)]
trait IntoPSTR {
    fn into_pstr(self) -> PSTR;
}

#[cfg(windows)]
trait IntoPCSTR {
    fn into_pcstr(self) -> PCSTR;
}

#[cfg(windows)]
trait IntoPCWSTR {
    fn into_pcwstr(self) -> PCWSTR;
}

#[cfg(windows)]
impl IntoPCWSTR for &str {
    fn into_pcwstr(self) -> PCWSTR {
        let encoded = self.encode_utf16().chain([0u16]).collect::<Vec<u16>>();

        PCWSTR(encoded.as_ptr())
    }
}

#[cfg(windows)]
impl IntoPWSTR for &str {
    fn into_pwstr(self) -> PWSTR {
        let mut encoded = self.encode_utf16().chain([0u16]).collect::<Vec<u16>>();

        PWSTR(encoded.as_mut_ptr())
    }
}

#[cfg(windows)]
impl IntoPSTR for &str {
    fn into_pstr(self) -> PSTR {
        let mut encoded = self
            .as_bytes()
            .iter()
            .cloned()
            .chain([0u8])
            .collect::<Vec<u8>>();

        PSTR(encoded.as_mut_ptr())
    }
}

#[cfg(windows)]
impl IntoPCSTR for &str {
    fn into_pcstr(self) -> PCSTR {
        let encoded = self
            .as_bytes()
            .iter()
            .cloned()
            .chain([0u8])
            .collect::<Vec<u8>>();

        PCSTR(encoded.as_ptr())
    }
}

#[cfg(windows)]
impl IntoPWSTR for String {
    fn into_pwstr(self) -> PWSTR {
        let mut encoded = self.encode_utf16().chain([0u16]).collect::<Vec<u16>>();

        PWSTR(encoded.as_mut_ptr())
    }
}

#[cfg(windows)]
fn command_ok(cmd: &mut Command) -> bool {
    cmd.status().ok().map_or(false, |s| s.success())
}

#[cfg(windows)]
fn command_output(cmd: &mut Command) -> String {
    str::from_utf8(&cmd.output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn main() {
    if std::env::var("DOCS_RS").is_ok() {
        return;
    }
    #[cfg(windows)]
    {
        // println!("cargo:rerun-if-changed=src/lib.rs");
        // println!("cargo:rerun-if-changed=src/native.rs");
        // println!("cargo:rerun-if-changed=src/csrc");
        println!("cargo:rerun-if-changed=src/");

        // let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        // let include_path = Path::new(&manifest_dir).join("include");
        // CFG.exported_header_dirs.push(&include_path);
        // CFG.exported_header_dirs.push(&Path::new(&manifest_dir));
        // Check if ConPTY is enabled
        let reg_entry = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";

        let major_version = command_output(
            Command::new("Reg")
                .arg("Query")
                .arg(&reg_entry)
                .arg("/v")
                .arg("CurrentMajorVersionNumber"),
        );
        let version_parts: Vec<&str> = major_version.split("REG_DWORD").collect();
        let major_version =
            i64::from_str_radix(version_parts[1].trim().trim_start_matches("0x"), 16).unwrap();

        let build_version = command_output(
            Command::new("Reg")
                .arg("Query")
                .arg(&reg_entry)
                .arg("/v")
                .arg("CurrentBuildNumber"),
        );
        let build_parts: Vec<&str> = build_version.split("REG_SZ").collect();
        let build_version = build_parts[1].trim().parse::<i64>().unwrap();

        println!("Windows major version: {:?}", major_version);
        println!("Windows build number: {:?}", build_version);

        let conpty_enabled;
        let kernel32_res = unsafe { GetModuleHandleW(&HSTRING::from("kernel32.dll")) };
        let kernel32 = kernel32_res.unwrap();

        let conpty = unsafe { GetProcAddress(kernel32, "CreatePseudoConsole".into_pcstr()) };
        match conpty {
            Some(_) => {
                conpty_enabled = "1";
                println!("cargo:rustc-cfg=feature=\"conpty\"")
            }
            None => {
                conpty_enabled = "0";
            }
        }

        println!("ConPTY enabled: {}", conpty_enabled);

        // Check if winpty is installed
        let mut cmd = Command::new("winpty-agent");
        let mut winpty_enabled = "0";
        if command_ok(cmd.arg("--version")) {
            // let winpty_path = cm
            winpty_enabled = "1";
            let winpty_version = command_output(cmd.arg("--version"));
            println!("Using Winpty version: {}", &winpty_version);

            let winpty_location = which("winpty-agent").unwrap();
            let winpty_path = winpty_location.parent().unwrap();
            let winpty_root = winpty_path.parent().unwrap();
            // let winpty_include = winpty_root.join("include");

            let winpty_lib = winpty_root.join("lib");

            println!(
                "cargo:rustc-link-search=native={}",
                winpty_lib.to_str().unwrap()
            );
            println!(
                "cargo:rustc-link-search=native={}",
                winpty_path.to_str().unwrap()
            );

            println!("cargo:rustc-cfg=feature=\"winpty\"")

            // CFG.exported_header_dirs.push(&winpty_include);
        }

        if winpty_enabled == "1" {
            println!("cargo:rustc-link-lib=dylib=winpty");
        }
    }
}
