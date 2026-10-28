#![cfg(feature="winpty")]

use std::ffi::OsString;
use std::env;
use std::thread::sleep;
use std::time::Duration;
use regex::Regex;

use winptyrs::{PTY, PTYArgs, PTYBackend, MouseMode, AgentConfig};

mod common;

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
        // Yield only when there was nothing to take: `read(false)`
        // returns immediately when nothing is queued, and spinning on
        // that starves the child on a small runner.
        if output_str.is_empty() {
            sleep(Duration::from_millis(10));
        }
    }

    assert!(regex.is_match(output_str));

    let echo_regex = Regex::new(".*echo \"This is a test stri.*").unwrap();
    pty.write(OsString::from("echo \"This is a test string\"")).unwrap();

    output_str = "";
    while !echo_regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
        // Yield only when there was nothing to take: `read(false)`
        // returns immediately when nothing is queued, and spinning on
        // that starves the child on a small runner.
        if output_str.is_empty() {
            sleep(Duration::from_millis(10));
        }
    }

    assert!(echo_regex.is_match(output_str));

    let out_regex = Regex::new(".*This is a test.*").unwrap();
    pty.write("\r\n".into()).unwrap();

    output_str = "";
    while !out_regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
        // Yield only when there was nothing to take: `read(false)`
        // returns immediately when nothing is queued, and spinning on
        // that starves the child on a small runner.
        if output_str.is_empty() {
            sleep(Duration::from_millis(10));
        }
    }

    assert!(out_regex.is_match(output_str));
    assert_ne!(pty.get_pid(), 0)
}

#[test]
fn set_size_winpty() {
    // Each wait is bounded and accumulating; see `common::read_until_into`.
    const BUDGET: Duration = Duration::from_secs(60);
    const MAX_PROBES: usize = 20;

    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from(r"C:\Windows\System32\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();

    let header_regex = Regex::new(r".*Width.*").unwrap();
    let num_regex = Regex::new(r"\s+(\d+)\s+(\d+).*").unwrap();
    let probe = "powershell -command \"&{(get-host).ui.rawui.WindowSize;}\"\r\n";

    /// Read the table, then take the size off its last numeric line.
    fn probe_size(
        pty: &PTY,
        probe: &str,
        header_regex: &Regex,
        num_regex: &Regex,
        budget: Duration,
    ) -> (i32, i32) {
        pty.write(probe.into()).unwrap();

        let mut collected = String::new();
        common::read_until_into(pty, &mut collected, header_regex, budget).unwrap();
        common::read_until_into(pty, &mut collected, num_regex, budget).unwrap();

        let mut cols = -1;
        let mut rows = -1;
        for part in collected.split("\r\n") {
            if let Some(caps) = num_regex.captures(part) {
                cols = caps[1].parse().unwrap();
                rows = caps[2].parse().unwrap();
            }
        }
        (cols, rows)
    }

    let (mut cols, mut rows) =
        probe_size(&pty, probe, &header_regex, &num_regex, BUDGET);

    if &env::var("CI").unwrap_or("0".to_owned()) == "0" {
        assert_eq!(rows, pty_args.rows);
        assert_eq!(cols, pty_args.cols);
    }

    pty.set_size(90, 30).unwrap();
    pty.write("cls\r\n".into()).unwrap();

    let mut probes = 0;
    while probes < 5 || (cols != 90 && rows != 30) {
        assert!(
            probes < MAX_PROBES,
            "window size still {cols}x{rows} after {probes} probes, expected 90x30"
        );

        (cols, rows) = probe_size(&pty, probe, &header_regex, &num_regex, BUDGET);
        probes += 1;
    }

    assert_eq!(cols, 90);
    assert_eq!(rows, 30);
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

/// A spaced `appname` must not corrupt the child's `argv`.
///
/// Regression test for andfoy/pywinpty#583, the defect #132's fix introduced. Because Win32
/// takes `argv[0]` from `lpCommandLine` even when `lpApplicationName` is
/// given, that fix prepends `appname` to `cmdline` - but it prepended it
/// unquoted, so a path with a space was split: the child saw
/// `argv[0] = "C:\Program"` and gained a bogus `"Files\..."` argument ahead of
/// its real ones.
///
/// Callers had no way around it. Quoting `appname` themselves makes
/// `CreateProcessW` reject `lpApplicationName` with `ERROR_INVALID_NAME`.
#[test]
fn spawn_spaced_appname_keeps_argv_winpty() {
    use std::time::{Duration, Instant};

    let Some(dir) = common::spaced_dir("winpty argv") else {
        eprintln!("could not create a directory whose path contains a space; skipping");
        return;
    };
    let Some(dumper) = common::build_argv_dumper(&dir) else {
        eprintln!("rustc not available to build the argv probe; skipping");
        return;
    };

    let pty_args = PTYArgs {
        cols: 200,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(
        dumper.clone().into_os_string(),
        Some(OsString::from(r#"--alpha "two words" beta"#)),
        None,
        None,
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut collected = String::new();
    while Instant::now() < deadline && common::parse_argv(&collected).len() < 4 {
        if let Ok(out) = pty.read(false) {
            collected.push_str(&out.to_string_lossy());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let argv = common::parse_argv(&collected);
    let expected = vec![
        dumper.to_string_lossy().into_owned(),
        "--alpha".to_string(),
        "two words".to_string(),
        "beta".to_string(),
    ];

    assert_eq!(
        argv, expected,
        "the spaced executable path was split across argv entries. Raw output: {collected:?}"
    );
}

/// `is_eof` has to report EOF on this backend too.
///
/// Companion to `is_eof_after_child_exits_conpty`. The two readers reach the
/// same flag by different routes - this one stops as soon as its own
/// `is_eof(process, conout)` check holds, rather than on a cancelled
/// overlapped read - so both are worth covering. See andfoy/pywinpty#423,
/// #519 and #565.
#[test]
fn is_eof_after_child_exits_winpty() {
    use std::time::{Duration, Instant};

    const MARKER: &str = "WINPTYRS_EOF_OK";

    let pty_args = PTYArgs {
        cols: 120,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(
        OsString::from(r"C:\Windows\System32\cmd.exe"),
        Some(OsString::from(format!("/c echo {MARKER}"))),
        None,
        None,
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut collected = String::new();
    let mut eof = false;

    while Instant::now() < deadline {
        match pty.read(false) {
            Ok(out) => collected.push_str(&out.to_string_lossy()),
            Err(err) => assert!(
                !err.is_empty(),
                "a read failed with an empty message, so callers cannot tell \
                 a closed stream from a broken one"
            ),
        }

        if pty.is_eof().unwrap() {
            eof = true;
            break;
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(
        collected.contains(MARKER),
        "the child's output never arrived. Got: {collected:?}"
    );
    assert!(
        eof,
        "is_eof() never reported EOF although the child exited. Got: {collected:?}"
    );
}

/// No process may outlive the PTY: not the child, not its grandchild, and not
/// the console host the pseudoconsole brought up.
///
/// The child is `cmd /c ping`, which lives long enough to be observed and
/// starts a grandchild of its own, then exits on its own account. Everything
/// that appeared while it ran has to be gone once the child has exited and the
/// PTY has been dropped.
#[test]
fn no_leftover_processes_after_child_exit_winpty() {
    use std::time::{Duration, Instant};

    let before = common::running_pids();

    let pty_args = PTYArgs {
        cols: 120,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut created = std::collections::BTreeMap::new();

    {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
        pty.spawn(
            OsString::from(r"C:\Windows\System32\cmd.exe"),
            Some(OsString::from("/c ping -n 3 127.0.0.1")),
            None,
            None,
        )
        .unwrap();

        // Accumulate over several samples: the host and the child do not
        // necessarily appear in the same instant.
        let watch_until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < watch_until {
            let _ = pty.read(false);
            created.extend(common::pty_processes_created(&before, &common::running_pids()));
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(
            !created.is_empty(),
            "no new process was observed at all, so this test would pass vacuously"
        );

        // Let the child finish on its own.
        let exit_deadline = Instant::now() + Duration::from_secs(30);
        while pty.is_alive().unwrap() {
            let _ = pty.read(false);
            assert!(Instant::now() < exit_deadline, "child never exited");
            std::thread::sleep(Duration::from_millis(50));
        }

        // Drain to EOF so teardown takes its normal course.
        let eof_deadline = Instant::now() + Duration::from_secs(10);
        while !pty.is_eof().unwrap() && Instant::now() < eof_deadline {
            let _ = pty.read(false);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // The PTY is now dropped. Give teardown a bounded moment, then require
    // that every process it created is gone. A PID is only counted as a
    // leftover if it is still present *and* still running the same image, so
    // that PID reuse cannot fail the test spuriously.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut leftover: Vec<(u32, String)>;

    loop {
        let now_running = common::running_pids();
        leftover = created
            .iter()
            .filter(|(pid, image)| now_running.get(pid) == Some(*image))
            .map(|(pid, image)| (*pid, image.clone()))
            .collect();

        if leftover.is_empty() || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    eprintln!(
        "LEAKCHECK winpty: {} process(es) created: {:?}",
        created.len(),
        created.values().collect::<Vec<_>>()
    );

    assert!(
        leftover.is_empty(),
        "process(es) left behind after the child exited and the PTY was dropped: {leftover:?}"
    );
}

/// Dropping the PTY while the child is still running must not orphan anything
/// either.
///
/// The child here would run for ~30 s if left alone, so anything still present
/// afterwards is a genuine leak rather than a timing artefact. Releasing the
/// pseudoconsole tears down the whole console session, which is why the child
/// and its grandchild go with it.
#[test]
fn no_leftover_processes_when_dropped_while_child_alive_winpty() {
    use std::time::{Duration, Instant};

    let before = common::running_pids();
    let pty_args = PTYArgs {
        cols: 120, rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut created = std::collections::BTreeMap::new();
    {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
        pty.spawn(
            OsString::from(r"C:\Windows\System32\cmd.exe"),
            Some(OsString::from("/c ping -n 30 127.0.0.1")),
            None, None,
        ).unwrap();

        let watch_until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < watch_until {
            let _ = pty.read(false);
            created.extend(common::pty_processes_created(&before, &common::running_pids()));
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(pty.is_alive().unwrap(), "child should still be running");
        assert!(
            !created.is_empty(),
            "no new process was observed at all, so this test would pass vacuously"
        );
        eprintln!(
            "LEAKCHECK winpty (dropped while alive): {:?}",
            created.values().collect::<Vec<_>>()
        );
    }

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut leftover: Vec<(u32, String)>;
    loop {
        let now = common::running_pids();
        leftover = created.iter()
            .filter(|(pid, image)| now.get(pid) == Some(*image))
            .map(|(pid, image)| (*pid, image.clone()))
            .collect();
        if leftover.is_empty() || Instant::now() >= deadline { break; }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        leftover.is_empty(),
        "process(es) left behind after dropping the PTY while the child was          still running: {leftover:?}"
    );
}

/// Reading and writing from different threads at the same time (#89).
///
/// `PTY` is `Send + Sync` and both `read` and `write` take `&self`, so it can
/// be shared through an `Arc`: writes are serialised by an internal mutex, and
/// reads are served from the reader thread's queue rather than touching the
/// pipe directly.
#[test]
fn concurrent_read_and_write_winpty() {
    use std::sync::Arc;
    use std::time::Instant;

    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PTY>();

    const MARKER: &str = "CONCURRENT_OK";

    let pty_args = PTYArgs {
        cols: 120, rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    // `spawn` needs `&mut self`, so share only afterwards.
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::WinPTY).unwrap();
    pty.spawn(OsString::from(r"C:\Windows\System32\cmd.exe"), None, None, None).unwrap();
    let pty = Arc::new(pty);

    let writer = {
        let pty = Arc::clone(&pty);
        std::thread::spawn(move || {
            // Write while the main thread is inside its read loop.
            sleep(Duration::from_millis(400));
            pty.write(OsString::from(format!("echo {MARKER}\r\n"))).unwrap();
        })
    };

    let mut collected = String::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    // Two occurrences: cmd echoes the typed line, then prints the result. One
    // alone would only prove the echo came back.
    while Instant::now() < deadline && collected.matches(MARKER).count() < 2 {
        match pty.read(false) {
            Ok(chunk) => {
                if chunk.is_empty() {
                    sleep(Duration::from_millis(10));
                }
                collected.push_str(&chunk.to_string_lossy());
            }
            Err(err) => panic!("read failed: {:?}", err.to_string_lossy()),
        }
    }

    writer.join().expect("writer thread");

    eprintln!("PROBE occurrences: {}", collected.matches(MARKER).count());
    assert!(
        collected.matches(MARKER).count() >= 2,
        "expected the echoed command and its output. Got: {collected:?}"
    );
}
