#![cfg(feature="conpty")]

use std::ffi::OsString;
use std::thread::sleep;
use std::time::Duration;
use std::{thread, time};
use regex::Regex;

use winptyrs::{PTY, PTYArgs, PTYBackend, MouseMode, AgentConfig};

mod common;

#[test]
#[ignore]
fn spawn_conpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();
    pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

    let ten_millis = time::Duration::from_millis(10);
    thread::sleep(ten_millis);
}

#[test]
fn read_write_conpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();
    pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

    let re_pattern: &str = r".*Microsoft Windows.*";
    let regex = Regex::new(re_pattern).unwrap();
    let mut output_str = "";
    let mut out: OsString;
    let mut tries = 0;

    sleep(Duration::from_millis(5000));

    pty.write("\r\n".into()).unwrap();

    while !regex.is_match(output_str) && tries < 100 {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
        // Yield only when there was nothing to take: `read(false)`
        // returns immediately when nothing is queued, and spinning on
        // that starves the child on a small runner.
        if output_str.is_empty() {
            sleep(Duration::from_millis(10));
        }
        println!("{:?}", output_str);
        tries += 1;
    }

    assert!(regex.is_match(output_str));

    let echo_regex = Regex::new(".*echo \"This is a test stri.*").unwrap();
    pty.write(OsString::from("echo \"This is a test string 😁\"")).unwrap();

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
        println!("{:?}", output_str);
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
        println!("Last: {:?}", output_str);
    }

    println!("!!!!!!!!!!!!!!!!!");
    assert!(out_regex.is_match(output_str));
    assert_ne!(pty.get_pid(), 0)
}

#[test]
fn set_size_conpty() {
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
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();

    pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

    let header_regex = Regex::new(r".*Width.*").unwrap();
    let num_regex = Regex::new(r".*\s+-*\s*-*\s+(\d+)\s+(\d+).*").unwrap();
    let probe = "powershell -command \"&{(get-host).ui.rawui.WindowSize;}\"\r\n";

    // Ask for the window size, then wait for the table header and its numbers.
    pty.write(probe.into()).unwrap();
    let mut collected = String::new();
    common::read_until_into(&pty, &mut collected, &header_regex, BUDGET).unwrap();
    common::read_until_into(&pty, &mut collected, &num_regex, BUDGET).unwrap();

    let (mut cols, mut rows) =
        common::last_size(&num_regex, &collected).expect("a window size");

    assert_eq!(rows, pty_args.rows);
    assert_eq!(cols, pty_args.cols);

    pty.set_size(90, 30).unwrap();

    for _ in 0..4 {
        pty.write("cls\r\n".into()).unwrap();
    }

    // The resize takes a moment to show up, so probe until it does - each
    // probe into a fresh buffer, so the size read back is the current one.
    let mut probes = 0;
    while probes < 5 || (cols != 90 && rows != 30) {
        assert!(
            probes < MAX_PROBES,
            "window size still {cols}x{rows} after {probes} probes, expected 90x30"
        );

        pty.write(probe.into()).unwrap();
        let mut collected = String::new();
        common::read_until_into(&pty, &mut collected, &header_regex, BUDGET).unwrap();
        common::read_until_into(&pty, &mut collected, &num_regex, BUDGET).unwrap();

        (cols, rows) = common::last_size(&num_regex, &collected).expect("a window size");
        probes += 1;
    }

    assert_eq!(cols, 90);
    assert_eq!(rows, 30);
}

#[test]
fn is_alive_exitstatus_conpty() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("C:\\Windows\\System32\\cmd.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();
    pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

    // sleep(Duration::from_millis(5000));

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
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(appname, None, None, None).unwrap();
    pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

    // sleep(Duration::from_millis(5000));

    pty.write("echo wait\r\n".into()).unwrap();
    assert!(pty.is_alive().unwrap());
    assert_eq!(pty.get_exitstatus().unwrap(), None);

    pty.write("exit\r\n".into()).unwrap();
    let _ = pty.wait_for_exit();

    assert!(!pty.is_alive().unwrap());
    assert_eq!(pty.get_exitstatus().unwrap(), Some(0))
}

#[test]
fn check_eof_output() {
    let pty_args = PTYArgs {
        cols: 80,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES
    };

    let appname = OsString::from("python.exe");
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(appname, Some(OsString::from("-c \"print(\';\'.join([str(i) for i in range(0, 2048)]))\"")), None, None).unwrap();
    assert!(pty.is_alive().unwrap());
    pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();
    // sleep(Duration::from_millis(5000));

    let mut collect_vec: Vec<String> = Vec::new();
    let mut valid = true;

    // Bounded, so that an `is_eof` that stops reporting EOF fails this test
    // instead of hanging it.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);

    while valid {
        assert!(
            std::time::Instant::now() < deadline,
            "never reached EOF; collected so far: {:?}",
            collect_vec.join("")
        );

        let out_wrapped = pty.read(true);
        match out_wrapped {
            Ok(out) =>{
                println!("{:?}", out);
                collect_vec.push(out.clone().into_string().unwrap());
                // Stop once there is nothing left to read *and* the stream has
                // ended. This used to be `&& !pty.is_eof()`, which is
                // backwards; it only terminated because `is_eof` could never
                // return `true`, so it stopped at the first empty read
                // instead.
                if out.is_empty() && pty.is_eof().unwrap() {
                    valid = false;
                }
            },
            Err(_) => {valid = false;}
        }
    }

    let output_str = collect_vec.join("");
    println!("{:?}", output_str);
    assert!(output_str.ends_with("2047\r\n"));

    println!("{:?}", output_str);
    let _ = pty.wait_for_exit();

}

#[test]
fn drop_timing_conpty() {
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

    let pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
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
/// Regression test for #132. ConPTY has always behaved correctly here; this pins the behaviour so the
/// two backends cannot drift apart again.
///
/// `python` is used deliberately rather than `cmd.exe`: cmd.exe scans its raw
/// command line for switches and so still honours a leading `/c` even when
/// `argv[0]` has been eaten, which masks the bug entirely. Python uses
/// C-runtime argv semantics, so if the first argument is consumed as
/// `argv[0]` it tries to open the `-c` code string as a *file* and fails with
/// "can't open file".
#[test]
fn spawn_cmdline_is_arguments_only_conpty() {
    use std::time::Instant;

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

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
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

/// A spaced `appname` must not corrupt the child's `argv` on ConPTY either.
///
/// Regression test for andfoy/pywinpty#583. This backend passes `lpApplicationName` as NULL,
/// so the concatenated command line is both the only thing naming the
/// executable and the source of `argv[0]`. Unquoted, a path with a space was
/// split exactly as on WinPTY - and here it also left `CreateProcessW` free to
/// walk its ambiguity search, which would prefer a `C:\Program.exe` over
/// `C:\Program Files\...`.
///
/// Both argument shapes are covered, because this backend concatenates
/// `appname` whether or not a `cmdline` was supplied: with no arguments at all
/// the child still saw a truncated `argv[0]`.
#[test]
fn spawn_spaced_appname_keeps_argv_conpty() {
    use std::time::Instant;

    let Some(dir) = common::spaced_dir("conpty argv") else {
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

    let cases: [(Option<OsString>, Vec<String>); 2] = [
        (
            Some(OsString::from(r#"--alpha "two words" beta"#)),
            vec![
                dumper.to_string_lossy().into_owned(),
                "--alpha".to_string(),
                "two words".to_string(),
                "beta".to_string(),
            ],
        ),
        (None, vec![dumper.to_string_lossy().into_owned()]),
    ];

    for (cmdline, expected) in cases {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
        pty.spawn(dumper.clone().into_os_string(), cmdline.clone(), None, None)
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut collected = String::new();
        while Instant::now() < deadline && common::parse_argv(&collected).len() < expected.len() {
            if let Ok(out) = pty.read(false) {
                collected.push_str(&out.to_string_lossy());
            }
            sleep(Duration::from_millis(50));
        }

        assert_eq!(
            common::parse_argv(&collected),
            expected,
            "spaced executable path was split across argv entries (cmdline: {cmdline:?}). \
             Raw output: {collected:?}"
        );
    }
}

/// `is_eof` has to report EOF once the child has gone and its output is
/// drained, and the stream has to wind down promptly.
///
/// Regression test for andfoy/pywinpty#423, #519 and #565. Three defects met
/// here:
///
/// * `is_eof` could never return `true`. It was `!(PeekNamedPipe(..).is_ok()
///   || is_alive || reader_atomic)`, and the peek succeeds for as long as the
///   handle is valid, so the first term alone always won.
/// * teardown waited a flat 5 s for an event ConPTY never raises on its own,
///   so a blocking read after the child exited took ~5.1 s to return against
///   0.059 s on WinPTY.
/// * the read then failed with an `NTSTATUS` cast straight into an `HRESULT`,
///   which no message table could resolve - so it surfaced as
///   `WinptyError('')`, an error with nothing in it.
#[test]
fn is_eof_after_child_exits_conpty() {
    use std::time::Instant;

    const MARKER: &str = "WINPTYRS_EOF_OK";

    let pty_args = PTYArgs {
        cols: 120,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(
        OsString::from(r"C:\Windows\System32\cmd.exe"),
        Some(OsString::from(format!("/c echo {MARKER}"))),
        None,
        None,
    )
    .unwrap();

    // Drain non-blocking until the marker has arrived and the child is gone.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut collected = String::new();
    let mut empty_errors = 0;
    while Instant::now() < deadline {
        match pty.read(false) {
            Ok(out) => collected.push_str(&out.to_string_lossy()),
            Err(err) => {
                if err.is_empty() {
                    empty_errors += 1;
                }
            }
        }
        if collected.contains(MARKER) && !pty.is_alive().unwrap() {
            break;
        }
        sleep(Duration::from_millis(50));
    }

    assert!(
        collected.contains(MARKER),
        "the child's output never arrived. Got: {collected:?}"
    );

    // A blocking read once the child has exited must not sit through a fixed
    // timeout waiting for an event that is not coming.
    let started = Instant::now();
    let last = pty.read(true);
    let blocked_for = started.elapsed();
    eprintln!("blocking read after child exit returned in {blocked_for:?}");

    if let Err(err) = &last {
        assert!(
            !err.is_empty(),
            "the final read failed with an empty message, so callers cannot tell \
             a closed stream from a broken one"
        );
    }

    assert!(
        blocked_for < Duration::from_secs(3),
        "a blocking read after the child exited took {blocked_for:?}; \
         teardown is waiting on a fixed deadline again"
    );

    // And EOF has to actually be reported.
    let eof_deadline = Instant::now() + Duration::from_secs(5);
    let mut eof = false;
    while Instant::now() < eof_deadline {
        if pty.is_eof().unwrap() {
            eof = true;
            break;
        }
        sleep(Duration::from_millis(50));
    }

    assert!(
        eof,
        "is_eof() never reported EOF although the child exited and its output was drained"
    );
    assert_eq!(
        empty_errors, 0,
        "{empty_errors} read(s) failed with an empty error message"
    );
}


/// Output still in flight when the child exits must be handed over in full.
///
/// This is the case teardown can get wrong in the dangerous direction. The
/// reader's pending read has to be cancelled at some point - ConPTY does not
/// always close the pipe on its own - but cancelling it while conhost is still
/// streaming would truncate the tail *and* report EOF, losing data silently.
///
/// Teardown therefore waits on the reader's progress rather than a deadline:
/// every delivery resets the idle timer, so a drain that is still moving is
/// never cut off. This test is the guard on that, and deliberately drains with
/// no sleep so it races the writer.
#[test]
fn trailing_output_survives_child_exit_conpty() {
    use std::time::Instant;

    const LINES: usize = 200_000;

    let Some(dir) = common::spaced_dir("conpty bulk") else {
        eprintln!("could not create a scratch directory; skipping");
        return;
    };
    let Some(writer) = common::build_bulk_writer(&dir) else {
        eprintln!("rustc not available to build the bulk writer; skipping");
        return;
    };

    let pty_args = PTYArgs {
        cols: 200,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    pty.spawn(
        writer.into_os_string(),
        Some(OsString::from(LINES.to_string())),
        None,
        None,
    )
    .unwrap();

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut collected = String::new();
    let mut eof = false;

    while Instant::now() < deadline {
        if let Ok(out) = pty.read(false) {
            collected.push_str(&out.to_string_lossy());
        }
        if pty.is_eof().unwrap() {
            eof = true;
            break;
        }
    }

    assert!(eof, "is_eof() never reported EOF");

    let cleaned = common::strip_ansi(&collected);
    let seen = cleaned.matches('E').count();
    let last = format!("L{:06}E", LINES - 1);

    eprintln!(
        "MEASURE bulk drain: {} bytes collected, {} line markers, last line present: {}",
        collected.len(),
        seen,
        cleaned.contains(&last)
    );

    assert!(
        cleaned.contains(&last),
        "the last line the child wrote before exiting ({last}) never arrived, \
         so the drain was cut short: {} bytes collected, {seen} markers seen",
        collected.len()
    );
}

/// A spaced `appname` must resolve to the executable asked for, not to
/// whatever `CreateProcessW`'s ambiguity search stumbles on first.
///
/// Regression test for andfoy/pywinpty#583, where spawning an executable from
/// a path containing a space failed with `ERROR_BAD_EXE_FORMAT` - surfacing in
/// pywinpty as `WinptyError: %1 is not a valid Win32 application`, with the
/// placeholder unsubstituted. The reporter correlated it with path length;
/// what actually matters is whether one of the search's shorter candidates
/// happens to exist, which `plant_bogus_candidate` arranges deterministically.
///
/// Quoting the `argv[0]` token removes the search altogether, so the path is
/// taken literally.
#[test]
fn spawn_spaced_appname_ignores_bogus_candidate_conpty() {
    use std::time::Instant;

    let Some((real, decoy)) = common::plant_bogus_candidate("winptyrs583") else {
        eprintln!("could not stage the ambiguity-search reproduction; skipping");
        return;
    };

    let pty_args = PTYArgs {
        cols: 200,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
    let spawned = pty.spawn(real.clone().into_os_string(), None, None, None);

    assert!(
        spawned.is_ok(),
        "spawn failed for a path containing a space, with {} planted ahead of it \
         in CreateProcessW's search order: {:?}",
        decoy.display(),
        spawned.err()
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut collected = String::new();
    while Instant::now() < deadline && common::parse_argv(&collected).is_empty() {
        if let Ok(out) = pty.read(false) {
            collected.push_str(&out.to_string_lossy());
        }
        sleep(Duration::from_millis(50));
    }

    assert_eq!(
        common::parse_argv(&collected),
        vec![real.to_string_lossy().into_owned()],
        "the wrong executable ran, or none did. Raw output: {collected:?}"
    );
}

/// Measurement, not an assertion - run with `--ignored --nocapture`.
///
/// `is_eof` is now an atomic load plus a channel emptiness check, where it
/// used to issue `PeekNamedPipe` and a process-state query on every call. This
/// reports the per-call cost in both states, and the cost of the loop shape
/// consumers actually write: a blocking read interleaved with an EOF check.
#[test]
#[ignore]
fn bench_eof_polling_conpty() {
    use std::time::Instant;

    const ITERS: usize = 1_000_000;
    const LINES: usize = 200_000;

    let pty_args = PTYArgs {
        cols: 200,
        rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    // --- per-call cost while the child is alive (the state a polling
    // --- consumer spends all its time in) ---
    {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
        pty.spawn(
            OsString::from(r"C:\Windows\System32\cmd.exe"),
            None,
            None,
            None,
        )
        .unwrap();

        let started = Instant::now();
        let mut eofs = 0usize;
        for _ in 0..ITERS {
            if pty.is_eof().unwrap() {
                eofs += 1;
            }
        }
        let elapsed = started.elapsed();
        eprintln!(
            "MICRO is_eof (child alive): {ITERS} calls in {elapsed:?} = {:.1} ns/call ({eofs} true)",
            elapsed.as_nanos() as f64 / ITERS as f64
        );
    }

    // --- per-call cost after EOF ---
    let Some(dir) = common::spaced_dir("conpty bench") else {
        eprintln!("could not create a scratch directory; skipping the rest");
        return;
    };
    let Some(writer) = common::build_bulk_writer(&dir) else {
        eprintln!("rustc not available; skipping the rest");
        return;
    };

    {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
        pty.spawn(
            writer.clone().into_os_string(),
            Some(OsString::from("50")),
            None,
            None,
        )
        .unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let _ = pty.read(false);
            if pty.is_eof().unwrap() {
                break;
            }
        }

        let started = Instant::now();
        let mut eofs = 0usize;
        for _ in 0..ITERS {
            if pty.is_eof().unwrap() {
                eofs += 1;
            }
        }
        let elapsed = started.elapsed();
        eprintln!(
            "MICRO is_eof (after EOF):   {ITERS} calls in {elapsed:?} = {:.1} ns/call ({eofs} true)",
            elapsed.as_nanos() as f64 / ITERS as f64
        );
    }

    // --- the real consumer loop: blocking read interleaved with is_eof ---
    {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
        pty.spawn(
            writer.into_os_string(),
            Some(OsString::from(LINES.to_string())),
            None,
            None,
        )
        .unwrap();

        let started = Instant::now();
        let mut iterations = 0usize;
        let mut bytes = 0usize;
        let deadline = Instant::now() + Duration::from_secs(120);

        // An empty read means the loop went round without the blocking read
        // having anything to hand over - i.e. a spin. A healthy loop should
        // have almost none.
        let mut empty_reads = 0usize;

        while !pty.is_eof().unwrap() {
            iterations += 1;
            match pty.read(true) {
                Ok(out) => {
                    let n = out.to_string_lossy().len();
                    if n == 0 {
                        empty_reads += 1;
                    }
                    bytes += n;
                }
                Err(_) => break,
            }
            assert!(Instant::now() < deadline, "loop did not terminate");
        }

        let elapsed = started.elapsed();
        eprintln!(
            "MACRO read(true)+is_eof: {bytes} bytes over {iterations} iterations in {elapsed:?} \
             ({:.1} MB/s, {:.1} us/iteration, {empty_reads} empty reads = {:.2}% spin)",
            bytes as f64 / 1e6 / elapsed.as_secs_f64(),
            elapsed.as_micros() as f64 / iterations as f64,
            100.0 * empty_reads as f64 / iterations as f64
        );
    }
}

/// No process may outlive the PTY: not the child, not its grandchild, and not
/// the console host the pseudoconsole brought up.
///
/// The child is `cmd /c ping`, which lives long enough to be observed and
/// starts a grandchild of its own, then exits on its own account. Everything
/// that appeared while it ran has to be gone once the child has exited and the
/// PTY has been dropped.
#[test]
fn no_leftover_processes_after_child_exit_conpty() {
    use std::time::Instant;

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
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
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
            sleep(Duration::from_millis(100));
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
            sleep(Duration::from_millis(50));
        }

        // Drain to EOF so teardown takes its normal course.
        let eof_deadline = Instant::now() + Duration::from_secs(10);
        while !pty.is_eof().unwrap() && Instant::now() < eof_deadline {
            let _ = pty.read(false);
            sleep(Duration::from_millis(20));
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
        sleep(Duration::from_millis(100));
    }

    eprintln!(
        "LEAKCHECK conpty: {} process(es) created: {:?}",
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
fn no_leftover_processes_when_dropped_while_child_alive_conpty() {
    use std::time::Instant;

    let before = common::running_pids();
    let pty_args = PTYArgs {
        cols: 120, rows: 25,
        mouse_mode: MouseMode::WINPTY_MOUSE_MODE_NONE,
        timeout: 10000,
        agent_config: AgentConfig::WINPTY_FLAG_COLOR_ESCAPES,
    };

    let mut created = std::collections::BTreeMap::new();
    {
        let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
        pty.spawn(
            OsString::from(r"C:\Windows\System32\cmd.exe"),
            Some(OsString::from("/c ping -n 30 127.0.0.1")),
            None, None,
        ).unwrap();

        let watch_until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < watch_until {
            let _ = pty.read(false);
            created.extend(common::pty_processes_created(&before, &common::running_pids()));
            sleep(Duration::from_millis(100));
        }
        assert!(pty.is_alive().unwrap(), "child should still be running");
        assert!(
            !created.is_empty(),
            "no new process was observed at all, so this test would pass vacuously"
        );
        eprintln!(
            "LEAKCHECK conpty (dropped while alive): {:?}",
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
        sleep(Duration::from_millis(200));
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
fn concurrent_read_and_write_conpty() {
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
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
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
