use glob::glob;
use std::env;
use std::env::consts::ARCH;
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
use num_traits::FromPrimitive;


// Variant names mirror the `IMAGE_FILE_MACHINE_*` constants in winnt.h
// verbatim, so that matching a PE header against this enum can be checked
// against Microsoft's documentation by eye. Renaming them to satisfy
// `upper_case_acronyms` would break that correspondence for no benefit.
#[allow(clippy::upper_case_acronyms)]
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


// Complete set of Win32 string-pointer conversions. Not all are called
// today; kept together so the set stays coherent, and build.rs is not
// part of the shipped crate.
#[cfg(windows)]
#[allow(dead_code)]
trait IntoPWSTR {
    fn into_pwstr(self) -> PWSTR;
}

#[cfg(windows)]
#[allow(dead_code)]
trait IntoPSTR {
    fn into_pstr(self) -> PSTR;
}

#[cfg(windows)]
#[allow(dead_code)]
trait IntoPCSTR {
    fn into_pcstr(self) -> PCSTR;
}

#[cfg(windows)]
#[allow(dead_code)]
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
    cmd.status().ok().is_some_and(|s| s.success())
}

#[cfg(windows)]
fn command_output(cmd: &mut Command) -> String {
    str::from_utf8(&cmd.output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// Version of the `Microsoft.Windows.Console.ConPTY` NuGet package to build
/// against.
///
/// This is pinned deliberately. It used to be unpinned, which meant every
/// build took whatever Microsoft had most recently published, so an upstream
/// release could break every build of this crate with no signal until
/// someone's tests hung - and the published crate carries no record of which
/// ConPTY it was validated against, so already-released versions changed
/// behaviour underneath their users too.
///
/// That is not hypothetical: 1.24.260512001 -> 1.25.260930003 changed when a
/// ConPTY client is released, which deadlocked teardown (see
/// `wait_for_exit_bounded` in `src/pty/base.rs`).
///
/// Bumping this is a deliberate change: run the full suite, including the
/// doc-tests, under `scripts/run-ci-local.ps1` before raising it.
///
/// Override with the `WINPTY_RS_CONPTY_VERSION` environment variable. Set it
/// to `latest` to resolve whatever NuGet currently serves - that is what the
/// scheduled canary workflow does, so an upstream ConPTY change shows up as a
/// failing canary rather than as a mystery hang weeks later. Dependabot
/// cannot do this for us: it only understands NuGet through project
/// manifests, not a version string in a Rust build script.
const CONPTY_NUGET_VERSION: &str = "1.25.260930003";

/// Environment variable that overrides [`CONPTY_NUGET_VERSION`].
const CONPTY_VERSION_ENV: &str = "WINPTY_RS_CONPTY_VERSION";

/// Resolve the ConPTY NuGet version to build against.
///
/// Returns `None` when the caller asked for `latest`, meaning no `-Version`
/// argument should be passed and NuGet picks the newest release.
fn conpty_nuget_version() -> Option<String> {
    println!("cargo:rerun-if-env-changed={CONPTY_VERSION_ENV}");
    match env::var(CONPTY_VERSION_ENV) {
        Ok(v) if v.eq_ignore_ascii_case("latest") => {
            println!("ConPTY version: latest (unpinned, requested via {CONPTY_VERSION_ENV})");
            None
        }
        Ok(v) if !v.trim().is_empty() => {
            println!("ConPTY version: {v} (from {CONPTY_VERSION_ENV})");
            Some(v)
        }
        _ => {
            println!("ConPTY version: {CONPTY_NUGET_VERSION} (pinned)");
            Some(CONPTY_NUGET_VERSION.to_owned())
        }
    }
}

fn get_output_path() -> PathBuf {
    //<root or manifest path>/target/<profile>/
    let manifest_dir_string = env::var("CARGO_MANIFEST_DIR").unwrap();
    let build_type = env::var("PROFILE").unwrap();
    let path = Path::new(&manifest_dir_string)
        .join("target")
        .join(build_type);
    path
}

/// Read the machine type out of a PE image's COFF header.
///
/// Returns `None` for anything that is not a readable Portable Executable,
/// including a file that simply is not there. This used to `unwrap` its way
/// through the parse, so a layout that had `winpty-agent.exe` without a
/// `winpty.dll` beside it aborted the build script instead of reporting that
/// winpty was unusable (see #38).
fn get_dll_target_arch(dll_path: &Path) -> Option<PEMachineType> {
    let file = File::open(dll_path).ok()?;
    let mut buf_reader = BufReader::new(file);
    buf_reader.seek(Start(0x3C)).ok()?;

    let mut off_buf: [u8; 4] = [0; 4];
    buf_reader.read_exact(&mut off_buf).ok()?;

    let pe_off = u32::from_ne_bytes(off_buf);
    // A PE offset below the DOS stub cannot be valid, and the subtraction
    // below would underflow and panic.
    let displ = pe_off.checked_sub(0x40)?;
    buf_reader.seek_relative(displ.into()).ok()?;

    let mut pe_buf: [u8; 6] = [0; 6];
    buf_reader.read_exact(&mut pe_buf).ok()?;

    if &pe_buf[0..2] != b"PE" {
        println!(
            "cargo:warning={} is not a Portable Executable",
            dll_path.display()
        );
        return None;
    }

    let machine_type_bytes: [u8; 2] = pe_buf[4..6].try_into().ok()?;
    let machine_type: u16 = u16::from_ne_bytes(machine_type_bytes);
    println!("DLL compilation target: {:#X}", machine_type);

    PEMachineType::from_u16(machine_type)
}

fn check_dll_arch_validity(dll_path: &Path, cur_arch: &str) -> bool {
    match get_dll_target_arch(dll_path) {
        Some(PEMachineType::AMD64) => cur_arch == "x64",
        Some(PEMachineType::ARM64) => cur_arch == "arm64",
        Some(PEMachineType::ARM64EC) => cur_arch == "x64",
        Some(PEMachineType::ARM64X) => cur_arch == "x64" || cur_arch == "arm64",
        _ => false
    }
}

/// Directories that may hold winpty's import library, in search order.
///
/// `LIB` is included because that is what the MSVC linker actually searches -
/// it ignores `PATH` entirely - so an install whose `winpty.lib` is only
/// reachable through `LIB` still links.
fn winpty_lib_search_dirs(winpty_bin: &Path, winpty_root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        winpty_root.join("lib"),
        winpty_bin.to_path_buf(),
        winpty_root.to_path_buf(),
    ];

    if let Some(lib_env) = env::var_os("LIB") {
        dirs.extend(env::split_paths(&lib_env));
    }

    dirs
}

/// Locate `winpty.lib`, returning the directory holding it.
///
/// Finding `winpty-agent.exe` on `PATH` does **not** mean winpty can be linked
/// against. Several programs ship the agent - and `winpty.dll` with it, since
/// both are needed at run time - without the import library, because they
/// redistribute winpty rather than provide an SDK. Neovim is the example in
/// #38. Linking is what actually has to succeed, so require the one file it
/// needs before claiming winpty is available.
fn find_winpty_import_lib(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .find(|dir| dir.join("winpty.lib").is_file())
        .cloned()
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

        let _current_path = env::current_dir().unwrap();

        let args = [
            "--in",
            "default",
            "lib/Windows.Wdk.winmd",
            "--out",
            "src/pty/conpty/win_bindings.rs",
            "--filter",
            "NtCreateNamedPipeFile",
            "--reference",
            "windows",
            // windows-bindgen 0.66 auto-inserts hardcoded references to
            // `windows_core::NTSTATUS` (and similar) at lib.rs:415, targeting an
            // unreleased future windows-rs layout. The latest published
            // windows-core (0.62.x) has no NTSTATUS, so the generated bindings
            // fail to compile. `--no-deps` skips those auto-inserts so our
            // `--reference windows` resolves NTSTATUS to
            // `windows::Win32::Foundation::NTSTATUS`.
            "--no-deps",
        ];

        _ = bindgen(args);

        // Check if ConPTY is enabled
        let reg_entry = "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";

        let major_version = command_output(
            Command::new("Reg")
                .arg("Query")
                .arg(reg_entry)
                .arg("/v")
                .arg("CurrentMajorVersionNumber"),
        );
        let version_parts: Vec<&str> = major_version.split("REG_DWORD").collect();
        let major_version =
            i64::from_str_radix(version_parts[1].trim().trim_start_matches("0x"), 16).unwrap();

        let build_version = command_output(
            Command::new("Reg")
                .arg("Query")
                .arg(reg_entry)
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

                let mut nuget_install = Command::new("nuget.exe");
                nuget_install
                    .current_dir(current_path.to_str().unwrap())
                    .arg("install")
                    .arg("Microsoft.Windows.Console.ConPTY");
                // Omitted entirely for `latest`, so NuGet resolves the
                // newest release (the canary workflow's case).
                if let Some(version) = conpty_nuget_version() {
                    nuget_install.arg("-Version").arg(version);
                }

                if command_ok(&mut nuget_install) {
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

        // Check if winpty is installed.
        //
        // Presence is established by finding `winpty-agent.exe` *and* the
        // import library the linker will need. See `find_winpty_import_lib`.
        println!("cargo:rerun-if-env-changed=LIB");

        let mut cmd = Command::new("winpty-agent");
        let mut winpty_enabled = false;
        let mut why_not: Option<String> = None;

        if command_ok(cmd.arg("--version")) {
            let winpty_version = command_output(cmd.arg("--version"));
            println!("Using Winpty version: {}", winpty_version);

            // `command_ok` succeeded, so the agent resolved on PATH.
            let winpty_location = which("winpty-agent").unwrap();
            let winpty_path = winpty_location.parent().unwrap();
            let winpty_root = winpty_path.parent().unwrap();
            // let winpty_include = winpty_root.join("include");

            let winpty_dll_path = winpty_path.join("winpty.dll");
            let valid_arch = check_dll_arch_validity(&winpty_dll_path, simplified_arch);
            let lib_dir =
                find_winpty_import_lib(&winpty_lib_search_dirs(winpty_path, winpty_root));

            match (valid_arch, lib_dir) {
                (true, Some(lib_dir)) => {
                    println!(
                        "cargo:rustc-link-search=native={}",
                        lib_dir.to_str().unwrap()
                    );
                    println!(
                        "cargo:rustc-link-search=native={}",
                        winpty_path.to_str().unwrap()
                    );
                    println!("cargo:rustc-cfg=feature=\"winpty\"");
                    winpty_enabled = true;
                }
                (false, _) => {
                    why_not = Some(format!(
                        "found winpty-agent at {} but {} is missing or built for \
                         another architecture than {}",
                        winpty_location.display(),
                        winpty_dll_path.display(),
                        simplified_arch
                    ));
                }
                (true, None) => {
                    why_not = Some(format!(
                        "found winpty-agent at {} but no winpty.lib to link against. \
                         Programs that redistribute winpty ship the agent and the DLL \
                         without the import library; install winpty's development \
                         files, or point LIB at the directory holding winpty.lib",
                        winpty_location.display()
                    ));
                }
            }

            // CFG.exported_header_dirs.push(&winpty_include);
        } else {
            why_not = Some("winpty-agent was not found on PATH".to_string());
        }

        if winpty_enabled {
            println!("cargo:rustc-link-lib=dylib=winpty");
        } else if let Some(reason) = why_not {
            // Say so up front. Asking for the feature and silently getting a
            // build without it - or a wall of `LNK2019: unresolved external
            // symbol winpty_*` - is what made this hard to diagnose.
            if env::var_os("CARGO_FEATURE_WINPTY").is_some() {
                panic!(
                    "the `winpty` feature was requested, but winpty cannot be linked: \
                     {reason}"
                );
            }
            println!("cargo:warning=winpty backend disabled: {reason}");
        }
    }
}
