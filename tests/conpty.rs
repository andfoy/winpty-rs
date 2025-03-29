#![cfg(feature="conpty")]

use std::ffi::OsString;
use std::{thread, time};
use regex::Regex;

use winptyrs::{PTY, PTYArgs, PTYBackend, MouseMode, AgentConfig};

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

    let re_pattern: &str = r".*Microsoft Windows.*";
    let regex = Regex::new(re_pattern).unwrap();
    let mut output_str = "";
    let mut out: OsString;
    let mut tries = 0;

    while !regex.is_match(output_str) && tries < 5 {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
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
        println!("{:?}", output_str);
    }

    assert!(echo_regex.is_match(output_str));

    let out_regex = Regex::new(".*This is a test.*").unwrap();
    pty.write("\r\n".into()).unwrap();

    output_str = "";
    while !out_regex.is_match(output_str) {
        out = pty.read(false).unwrap();
        output_str = out.to_str().unwrap();
        println!("{:?}", output_str);
    }

    println!("!!!!!!!!!!!!!!!!!");
    assert!(out_regex.is_match(output_str));
    assert_ne!(pty.get_pid(), 0)
}

#[test]
fn set_size_conpty() {
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

    assert_eq!(rows, pty_args.rows);
    assert_eq!(cols, pty_args.cols);

    pty.set_size(90, 30).unwrap();

    // if &env::var("CI").unwrap_or("0".to_owned()) == "1" {
    //     return;
    // }

    pty.write("cls\r\n".into()).unwrap();
    pty.write("cls\r\n".into()).unwrap();
    pty.write("cls\r\n".into()).unwrap();
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

        println!("{:?}", output_str);

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
    let mut pty = PTY::new_with_backend(&pty_args, PTYBackend::ConPTY).unwrap();
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

    let mut collect_vec: Vec<String> = Vec::new();
    let mut valid = true;

    while valid {
        let out_wrapped = pty.read(false);
        match out_wrapped {
            Ok(out) => collect_vec.push(out.into_string().unwrap()),
            Err(_) => {valid = false;}
        }
    }

    let output_str = collect_vec.join("");
    assert!(output_str.ends_with("2047\r\n"));

    println!("{:?}", output_str);
    let _ = pty.wait_for_exit();

}
