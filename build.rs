use glob::glob;
use std::env;
use std::env::consts::ARCH;
use std::i64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs::File;
use std::str;
use std::io::BufReader;
use std::io::prelude::*;
use std::io::SeekFrom::Start;
use which::which;
#[cfg(windows)]
use windows::core::{PCSTR, PCWSTR, PSTR, PWSTR};
#[cfg(windows)]
use windows_bindgen::bindgen;

use enum_primitive_derive::Primitive;
use num_traits::{FromPrimitive, ToPrimitive};


#[derive(Debug, Eq, PartialEq, Primitive)]
enum PEMachineType {
    UNKNOWN	= 0x0,
    ALPHA	= 0x184,
    ALPHA64	= 0x284,
    AM33	= 0x1d3,
    AMD64	= 0x8664,
    ARM	= 0x1c0,
    ARM64	= 0xaa64,
    ARM64EC	= 0xA641,
    ARM64X	= 0xA64E,
    ARMNT	= 0x1c4,
    EBC	= 0xebc,
    I386	= 0x14c,
    IA64	= 0x200,
    LOONGARCH32	= 0x6232,
    LOONGARCH64	= 0x6264,
    M32R	= 0x9041,
    MIPS16	= 0x266,
    MIPSFPU	= 0x366,
    MIPSFPU16	= 0x466,
    POWERPC	= 0x1f0,
    POWERPCFP	= 0x1f1,
    R3000BE	= 0x160,
    R3000	= 0x162,
    R4000	= 0x166,
    R10000	= 0x168,
    RISCV32	= 0x5032,
    RISCV64	= 0x5064,
    RISCV128	= 0x5128,
    SH3	= 0x1a2,
    SH3DSP	= 0x1a3,
    SH4	= 0x1a6,
    SH5	= 0x1a8,
    THUMB	= 0x1c2,
    WCEMIPSV2	= 0x169,
}


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

fn get_output_path() -> PathBuf {
    //<root or manifest path>/target/<profile>/
    let manifest_dir_string = env::var("CARGO_MANIFEST_DIR").unwrap();
    let build_type = env::var("PROFILE").unwrap();
    let path = Path::new(&manifest_dir_string)
        .join("target")
        .join(build_type);
    return PathBuf::from(path);
}

fn get_dll_target_arch(dll_path: PathBuf) -> PEMachineType {
    let file = File::open(dll_path).unwrap();
    let mut buf_reader = BufReader::new(file);
    buf_reader.seek(Start(0x3C)).unwrap();

    let mut off_buf: [u8; 4] = [0; 4];
    buf_reader.read(&mut off_buf).unwrap();

    let pe_off = u32::from_ne_bytes(off_buf);
    let displ = pe_off - 0x40;
    buf_reader.seek_relative(displ.into()).unwrap();

    let mut pe_buf: [u8; 6] = [0; 6];
    buf_reader.read(&mut pe_buf).unwrap();

    assert_eq!(&pe_buf[0..2], b"PE", "File is not a Portable Executable");

    let machine_type_bytes: [u8; 2] = pe_buf[4..6].try_into().unwrap();
    let machine_type: u16 = u16::from_ne_bytes(machine_type_bytes);
    println!("DLL compilation target: {:#X}", machine_type);

    PEMachineType::from_u16(machine_type).unwrap()
}

fn check_dll_arch_validity(dll_path: PathBuf, cur_arch: &str) -> bool {
    let target_arch = get_dll_target_arch(dll_path);

    match target_arch {
        PEMachineType::AMD64 => cur_arch == "x64",
        PEMachineType::ARM64 => cur_arch == "arm64",
        PEMachineType::ARM64EC => cur_arch == "x64",
        PEMachineType::ARM64X => cur_arch == "x64" || cur_arch == "arm64",
        _ => false
    }
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
        let simplified_arch = match ARCH {
            "x86_64" => "x64",
            "arm" => "arm64",
            "aarch64" => "arm64",
            _ => ARCH,
        };

        println!("cargo:rerun-if-changed=src/");

        // let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        // let include_path = Path::new(&manifest_dir).join("include");
        // CFG.exported_header_dirs.push(&include_path);
        // CFG.exported_header_dirs.push(&Path::new(&manifest_dir));
        let conpty_enabled;

        let current_path = env::current_dir().unwrap();

        let args = [
            "--in",
            "default",
            "lib/Windows.Wdk.winmd",
            "--out",
            "src/pty/conpty/win_bindings.rs",
            "--filter",
            "NtCreateNamedPipeFile",
            "--reference",
            "windows"
        ];

        _ = bindgen(args);

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

        // let conpty_enabled;
        // let kernel32_res = unsafe { GetModuleHandleW(&HSTRING::from("kernel32.dll")) };
        // let kernel32 = kernel32_res.unwrap();

        // let conpty = unsafe { GetProcAddress(kernel32, "CreatePseudoConsole".into_pcstr()) };
        match major_version >= 10 && build_version >= 2004 {
            true => {
                conpty_enabled = "1";
                println!("cargo:rustc-cfg=feature=\"conpty\"")
            }
            false => {
                conpty_enabled = "0";
            }
        }

        println!("ConPTY enabled: {}", conpty_enabled);
        println!("Output path: {}", get_output_path().to_str().unwrap());
        // println!("ConPTY binaries found locally: {}", conpty_locally_enabled);

        if conpty_enabled == "1" {
            // Check if local ConPTY binaries are available

            use std::fs;
            let current_path = env::current_dir().unwrap();
            // let lib_path = current_path.join("lib");
            let lib_path = get_output_path();
            if !fs::exists(&lib_path).unwrap() {
                fs::create_dir_all(&lib_path).unwrap();
            }


            let mut binaries_found = true;
            for bin_name in ["conpty.lib", "conpty.dll", "OpenConsole.exe"] {
                let bin_path = lib_path.join(bin_name);
                binaries_found = binaries_found && bin_path.exists();
            }

            let mut nuget;

            if !binaries_found {
                nuget = Command::new("nuget.exe");
                let nuget_found = command_ok(nuget.arg("help"));

                if !nuget_found {
                    panic!("NuGet is required to build winpty-rs");
                }

                if command_ok(
                     Command::new("nuget.exe")
                        .current_dir(current_path.to_str().unwrap())
                        .arg("install")
                        .arg("Microsoft.Windows.Console.ConPTY"),
                ) {
                    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
                    let manifest_path = PathBuf::from(Path::new(&manifest_dir));
                    for path in glob(
                        manifest_path
                            .join("Microsoft.Windows.Console.ConPTY*")
                            .to_str()
                            .unwrap(),
                    )
                    .unwrap()
                    {
                        match path {
                            Ok(folder) => {
                                use std::fs;

                                println!("{:?}", folder);
                                println!("{:?}", get_output_path());
                                let openconsole = folder
                                    .join("build")
                                    .join("native")
                                    .join("runtimes")
                                    .join(simplified_arch)
                                    .join("OpenConsole.exe");

                                let binaries_path = folder
                                    .join("runtimes")
                                    .join(format!("win-{}", simplified_arch));
                                let dll_path = binaries_path.join("native").join("conpty.dll");
                                let lib_orig =
                                    binaries_path.join("lib").join("uap10.0").join("conpty.lib");

                                let openconsole_dst = get_output_path().join("OpenConsole.exe");
                                let dll_dst = get_output_path().join("conpty.dll");
                                let lib_dst = get_output_path().join("conpty.lib");

                                fs::copy(openconsole, openconsole_dst).unwrap();
                                fs::copy(dll_path, dll_dst).unwrap();
                                fs::copy(lib_orig, lib_dst).unwrap();
                                binaries_found = true;

                                fs::remove_dir_all(folder).unwrap();
                            }
                            Err(err) => panic!("{:?}", err),
                        }
                    }
                }
            }

            // let conpty_enabled;
            if binaries_found {
                println!("cargo:rustc-cfg=feature=\"conpty\"");
                println!("cargo:rustc-cfg=feature=\"conpty_local\"");

                println!(
                    "cargo:rustc-link-search=native={}",
                    lib_path.to_str().unwrap()
                );
                println!(
                    "cargo:rustc-link-search=native={}",
                    lib_path.to_str().unwrap()
                );

                println!("cargo:rustc-link-lib=dylib=conpty");
            }
        }

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
            let winpty_dll_path = winpty_path.join("winpty.dll");

            let valid_arch = check_dll_arch_validity(winpty_dll_path, simplified_arch);

            winpty_enabled = match valid_arch {
                true => "1",
                false => "0"
            };

            if valid_arch {
                println!(
                    "cargo:rustc-link-search=native={}",
                    winpty_lib.to_str().unwrap()
                );
                println!(
                    "cargo:rustc-link-search=native={}",
                    winpty_path.to_str().unwrap()
                );
            }

            println!("cargo:rustc-cfg=feature=\"winpty\"");
            // CFG.exported_header_dirs.push(&winpty_include);
        }

        if winpty_enabled == "1" {
            println!("cargo:rustc-link-lib=dylib=winpty");
        }
    }
}
