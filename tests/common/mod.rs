//! Helpers shared by the backend test suites.

#![allow(dead_code)]

use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Build a self-contained helper that prints its own `argv`, one entry per
/// line wrapped in angle brackets, into `dir`.
///
/// Compiled on the fly with `rustc` rather than shipped as a fixture: the
/// point of the `argv` tests is to run the probe from a path containing a
/// space, and a freshly built binary can be placed wherever the test needs
/// it. Unlike `python.exe` it is also self-contained, so copying it somewhere
/// odd does not drag a standard library along with it.
///
/// Returns `None` if `rustc` is not on PATH, so the suite degrades to a skip
/// rather than a failure.
pub fn build_argv_dumper(dir: &Path) -> Option<PathBuf> {
    let src = dir.join("argv_dump.rs");
    let exe = dir.join("argv_dump.exe");

    std::fs::write(
        &src,
        "fn main() { for arg in std::env::args() { println!(\"<{arg}>\"); } }\n",
    )
    .ok()?;

    let status = std::process::Command::new("rustc")
        .arg("-O")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .status()
        .ok()?;

    (status.success() && exe.is_file()).then_some(exe)
}

/// Build a helper that writes `n` numbered lines and exits immediately, into
/// `dir`.
///
/// Used to check that output still in flight when the child exits is handed
/// over in full. The writes are buffered and explicitly flushed so that the
/// producer is not itself the thing dropping data - unbuffered single writes
/// hit an unrelated CPython defect (cpython#86210), and the point here is to
/// exercise *our* drain.
///
/// Returns `None` if `rustc` is not on PATH.
pub fn build_bulk_writer(dir: &Path) -> Option<PathBuf> {
    let src = dir.join("bulk_writer.rs");
    let exe = dir.join("bulk_writer.exe");

    std::fs::write(
        &src,
        r#"use std::io::Write;

fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for i in 0..n {
        writeln!(out, "L{i:06}E").unwrap();
    }
    out.flush().unwrap();
}
"#,
    )
    .ok()?;

    let status = std::process::Command::new("rustc")
        .arg("-O")
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .status()
        .ok()?;

    (status.success() && exe.is_file()).then_some(exe)
}

/// Strip the escape sequences a pseudoconsole interleaves with the child's
/// output, so assertions can look at the text alone.
pub fn strip_ansi(raw: &str) -> String {
    let csi = Regex::new(r"\x1b\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]").unwrap();
    let osc = Regex::new(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?").unwrap();
    let other = Regex::new(r"\x1b[\x20-\x7e]").unwrap();

    let out = csi.replace_all(raw, "");
    let out = osc.replace_all(&out, "");
    other.replace_all(&out, "").into_owned()
}

/// Recover the `argv` entries that `build_argv_dumper`'s helper printed.
pub fn parse_argv(raw: &str) -> Vec<String> {
    let cleaned = strip_ansi(raw);
    Regex::new(r"<([^>]*)>")
        .unwrap()
        .captures_iter(&cleaned)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// A directory whose path contains a space, recreated empty.
pub fn spaced_dir(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("winptyrs {name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;
    // Guard against a `TMP` that is itself quoted or otherwise surprising:
    // the test is meaningless if the path has no space to split on.
    dir.to_str()?.contains(' ').then_some(dir)
}

/// Lay out the shape that makes `CreateProcessW`'s ambiguity search misfire.
///
/// With `lpApplicationName` NULL and an unquoted command line, Win32 walks the
/// path one space at a time, appending `.exe` to each prefix: for
/// `...\base\a b\real.exe` it tries `...\base\a.exe` *first*. If something
/// exists there, it is launched - or, if it is not a valid image, the spawn
/// fails with `ERROR_BAD_EXE_FORMAT` and never reaches the real executable.
///
/// Returns `(real_exe, decoy)`: the probe to spawn, and the bogus candidate
/// planted ahead of it.
pub fn plant_bogus_candidate(base_name: &str) -> Option<(PathBuf, PathBuf)> {
    let base = std::env::temp_dir().join(base_name);
    let _ = std::fs::remove_dir_all(&base);

    // `a b` so that the first space-split prefix of the full path is
    // `<base>\a`, which Win32 will try as `<base>\a.exe`.
    let exe_dir = base.join("a b");
    std::fs::create_dir_all(&exe_dir).ok()?;

    let real = build_argv_dumper(&exe_dir)?;

    // Not a valid PE image, so reaching it is an unmistakable failure rather
    // than a silently different process.
    let decoy = base.join("a.exe");
    std::fs::write(&decoy, b"this is not a valid Win32 application\n").ok()?;

    // The reproduction is meaningless unless the path really does have a space
    // for the search to split on.
    real.to_str()?.contains(' ').then_some((real, decoy))
}

/// Image names a PTY is expected to bring into existence, beyond the child
/// itself: the console host (`conhost.exe`, or `OpenConsole.exe` when the
/// NuGet ConPTY is linked) and WinPTY's agent.
pub const PTY_HELPER_IMAGES: &[&str] = &[
    "conhost.exe",
    "openconsole.exe",
    "winpty-agent.exe",
    // The child used by the leak tests, and the grandchild it starts - a
    // grandchild left running would be an orphan just as surely as a leaked
    // console host.
    "cmd.exe",
    "ping.exe",
];

/// Every running process, as `pid -> lower-cased image name`.
///
/// Shelled out to `tasklist` rather than enumerated directly, because an
/// integration test cannot see the library's own `windows` dependency.
pub fn running_pids() -> BTreeMap<u32, String> {
    let mut map = BTreeMap::new();

    let Ok(output) = std::process::Command::new("tasklist")
        .args(["/NH", "/FO", "CSV"])
        .output()
    else {
        return map;
    };

    for line in String::from_utf8_lossy(&output.stdout).lines() {
        // `"image.exe","1234","Console","1","5,000 K"`
        let fields: Vec<&str> = line.trim().trim_matches('"').split("\",\"").collect();
        if fields.len() < 2 {
            continue;
        }
        if let Ok(pid) = fields[1].trim().parse::<u32>() {
            map.insert(pid, fields[0].to_ascii_lowercase());
        }
    }

    map
}

/// The processes present in `during` but not in `before`, restricted to images
/// a PTY is expected to create.
///
/// Restricted deliberately: unrelated processes start and stop on a live
/// machine all the time, and attributing those to the PTY would make the test
/// flaky.
pub fn pty_processes_created(
    before: &BTreeMap<u32, String>,
    during: &BTreeMap<u32, String>,
) -> BTreeMap<u32, String> {
    during
        .iter()
        .filter(|(pid, image)| {
            !before.contains_key(pid) && PTY_HELPER_IMAGES.contains(&image.as_str())
        })
        .map(|(pid, image)| (*pid, image.clone()))
        .collect()
}
