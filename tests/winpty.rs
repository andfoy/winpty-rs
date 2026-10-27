#![cfg(feature="winpty")]

use std::ffi::OsString;
use std::env;
use regex::Regex;

use winptyrs::{PTY, PTYArgs, PTYBackend, MouseMode, AgentConfig};

#[test]
fn spawn_winpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();
}

#[test]
fn read_write_winpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();

    let regex = Regex::new(r".*Microsoft Windows.*").unwrap();
    let mut output_str = "";
    let mut out: OsString;

    while !regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
    }

    assert!(regex.is_match(output_str));

    let echo_regex = Regex::new(".*echo \"This is a test stri.*").unwrap();
    pty.write(OsString::from("echo \"This is a test string\"")).unwrap();

    output_str = "";
    while !echo_regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
    }

    assert!(echo_regex.is_match(output_str));

    let out_regex = Regex::new(".*This is a test.*").unwrap();
    pty.write("\r\n".into()).unwrap();

    output_str = "";
    while !out_regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
    }

    assert!(out_regex.is_match(output_str));
    assert_ne!(pty.get_pid(), 0)
}

#[test]
fn set_size_winpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();

    pty.write("powershell -command \"&{(get-host).ui.rawui.WindowSize;}\"\r\n".into()).unwrap();
    let regex = Regex::new(r".*Width.*").unwrap();
    let mut output_str = "";
    let mut out: OsString;

    while !regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
    }

    let parts: Vec<&str> = output_str.split("\r\n").collect();
    let num_regex = Regex::new(r"\s+(\d+)\s+(\d+).*").unwrap();
    let mut rows: i32 = -1;
    let mut cols: i32 = -1;
    for part in parts {
        if num_regex.is_match(part) {
            for cap in num_regex.captures_iter(part) {
                cols = cap[1].parse().unwrap();
                rows = cap[2].parse().unwrap();
            }
        }
    }

    if &env::var("CI").unwrap_or("0".to_owned()) == "0" {
        assert_eq!(rows, pty_args.rows);
        assert_eq!(cols, pty_args.cols);
    }

    pty.set_size(90, 30).unwrap();
    pty.write("cls\r\n".into()).unwrap();

    let mut count = 0;
    while count < 5 || (cols != 90 && rows != 30) {
        pty.write("powershell -command \"&{(get-host).ui.rawui.WindowSize;}\"\r\n".into()).unwrap();
        let regex = Regex::new(r".*Width.*").unwrap();
        let mut output_str = "";
        let mut out: OsString;

        while !regex.is_match(output_str) {
            out = pty.read(false).unwrap();
            output_str = out.to_str().unwrap();
        }

        let parts: Vec<&str> = output_str.split("\r\n").collect();
        let num_regex = Regex::new(r"\s+(\d+)\s+(\d+).*").unwrap();
        for part in parts {
            if num_regex.is_match(part) {
                for cap in num_regex.captures_iter(part) {
                    cols = cap[1].parse().unwrap();
                    rows = cap[2].parse().unwrap();
                }
            }
        }

        count += 1;
    }
    // if &env::var("CI").unwrap_or("0".to_owned()) == "0" {
        assert_eq!(cols, 90);
        assert_eq!(rows, 30);
    // }
}

#[test]
fn is_alive_exitstatus_winpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();

    pty.write("echo wait\r\n".into()).unwrap();
    assert!(pty.is_alive().unwrap());
    assert_eq!(pty.get_exitstatus().unwrap(), None);

    pty.write("exit\r\n".into()).unwrap();
    while pty.is_alive().unwrap() {
        ()
    }
    assert!(!pty.is_alive().unwrap());
    assert_eq!(pty.get_exitstatus().unwrap(), Some(0))
}


#[test]
fn wait_for_exit() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();

    pty.write("echo wait\r\n".into()).unwrap();
    assert!(pty.is_alive().unwrap());
    assert_eq!(pty.get_exitstatus().unwrap(), None);

    pty.write("exit\r\n".into()).unwrap();
    let _ = pty.wait_for_exit();

    assert!(!pty.is_alive().unwrap());
    assert_eq!(pty.get_exitstatus().unwrap(), Some(0))
}

#[test]
fn drop_timing_winpty() {
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::{ Duration, Instant };

    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    let (tx, rx) = channel::<Instant>();

    thread::spawn(move || {
        let start = Instant::now();
        drop(pty);
        let _ = tx.send(start);
    });

    let timeout = Duration::from_secs(2);
    match rx.recv_timeout(timeout) {
        Ok(start) => {
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 1000,
                "drop took {}ms, expected < 1000ms",
                elapsed.as_millis()
            );
            println!("drop completed in {}ms", elapsed.as_millis());
        }
        Err(_) => {
            panic!(
                "drop timed out after {} seconds, possible infinite loop detected",
                timeout.as_secs()
            );
        }
    }
}

/// Locate `python.exe` on PATH, returning its full path.
///
/// The WinPTY backend passes `appname` as `CreateProcessW`'s
/// `lpApplicationName`, and Windows does not search PATH for that argument,
/// so a bare "python.exe" would not resolve. Tests that need a real argv
/// probe therefore have to resolve it themselves.
fn find_python() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("python.exe"))
            .find(|candidate| candidate.is_file())
    })
}

/// `cmdline` carries arguments only; `appname` supplies `argv[0]`.
///
/// Regression test for #132. This is the case the bug lived in: every other WinPTY test here passes
/// `cmdline = None`, and pywinpty's fixture spawns a bare `cmd`, so this
/// backend combined with arguments was never exercised anywhere.
///
/// `python` is used deliberately rather than `cmd.exe`: cmd.exe scans its raw
/// command line for switches and so still honours a leading `/c` even when
/// `argv[0]` has been eaten, which masks the bug entirely. Python uses
/// C-runtime argv semantics, so if the first argument is consumed as
/// `argv[0]` it tries to open the `-c` code string as a *file* and fails with
/// "can't open file".
#[test]
fn spawn_cmdline_is_arguments_only_winpty() {
    use std::time::{Duration, Instant};

    const MARKER: &str = "WINPTYRS_ARGV_OK";

    let Some(python) = find_python() else {
        eprintln!("python.exe not found on PATH; skipping");
        return;
    };

    let pty_args = PTYArgs {
        cols: 120,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(
        python.into_os_string(),
        Some(OsString::from(r#"-c "print('WINPTYRS_ARGV_OK')""#)),
        None,
        None,
    )
    .unwrap();
    let _ = pty.write(OsString::from("[?1;0c[0;0R"));

    // Bounded: never spin waiting for output that may never arrive.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut collected = String::new();
    while Instant::now() < deadline
        && !collected.contains(MARKER)
        && !collected.contains("can't open file")
    {
        if let Ok(out) = pty.read(false) {
            collected.push_str(&out.to_string_lossy());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        !collected.contains("can't open file"),
        "the first argument was consumed as argv[0], so python received the          -c code string as a filename. `cmdline` must mean arguments only,          with `appname` supplying argv[0]. Got: {collected:?}"
    );
    assert!(
        collected.contains(MARKER),
        "python did not run the -c code. Got: {collected:?}"
    );
}
