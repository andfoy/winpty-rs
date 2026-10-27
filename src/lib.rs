//! Create and spawn processes inside a pseudoterminal in Windows.
//!
//! This crate provides an abstraction over different backend implementations to spawn PTY processes in Windows.
//! Right now this library supports using [`WinPTY`] and [`ConPTY`].
//!
//! The abstraction is represented through the [`PTY`] struct, which declares methods to initialize, spawn, read,
//! write and get diverse information about the state of a process that is running inside a pseudoterminal.
//!
//! [`WinPTY`]: https://github.com/rprichard/winpty
//! [`ConPTY`]: https://docs.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session


#[macro_use]
extern crate enum_primitive_derive;
extern crate num_traits;

pub mod pty;
// mod pty_spawn;
pub use pty::{PTY, PTYArgs, PTYBackend, MouseMode, AgentConfig};

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use std::thread::sleep;
    use std::time::Duration;
    use std::ffi::OsString;

    #[test]
    fn test_write_performance() {
        // Initialize PTY with default arguments
        let args = PTYArgs { cols: 80, rows: 24, ..Default::default() };
        let mut pty = PTY::new_with_backend(&args, PTYBackend::ConPTY).unwrap();

        // Spawn cmd.exe
        let cmd = OsString::from("c:\\windows\\system32\\cmd.exe");
        pty.spawn(cmd, None, None, None).unwrap();
        pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

        // Wait for process to start
        sleep(Duration::from_millis(1000));

        // Test data
        let test_data = OsString::from("echo test\r\n");
        let iterations = 100;
        let mut total_time = Duration::from_secs(0);

        // Perform write performance test
        for i in 0..iterations {
            let start = Instant::now();
            pty.write(test_data.clone()).unwrap();
            let duration = start.elapsed();
            total_time += duration;
            println!("Write {}: {:?}", i + 1, duration);
        }

        // Calculate and print statistics
        let avg_time = total_time.as_secs_f64() / iterations as f64;
        println!("\nWrite Performance Test Results:");
        println!("Total time: {:?}", total_time);
        println!("Average time per write: {:.2}ms", avg_time * 1000.0);
        println!("Total writes: {}", iterations);
    }

    #[test]
    fn test_read_performance() {
        // Initialize PTY with default arguments
        let args = PTYArgs { cols: 80, rows: 24, ..Default::default() };
        let mut pty = PTY::new_with_backend(&args, PTYBackend::ConPTY).unwrap();

        // Spawn cmd.exe with a command that produces continuous output
        let cmd = OsString::from("c:\\windows\\system32\\cmd.exe");
        pty.spawn(cmd, Some("/c echo test".into()), None, None).unwrap();
        pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

        // Wait for process to start
        sleep(Duration::from_millis(1000));

        // Test parameters
        let iterations = 100;
        let mut total_time = Duration::from_secs(0);
        let mut total_bytes = 0;
        let mut successful_reads = 0;

        // Perform read performance test
        for i in 0..iterations {
            let start = Instant::now();
            match pty.read(false) {
                Ok(data) => {
                    let duration = start.elapsed();
                    total_time += duration;
                    total_bytes += data.len();
                    successful_reads += 1;
                    println!("Read {}: {:?}, bytes: {}", i + 1, duration, data.len());
                }
                Err(e) => {
                    println!("Read {} failed: {:?}", i + 1, e);
                }
            }
        }

        // Calculate and print statistics
        let avg_time = if successful_reads > 0 {
            total_time.as_secs_f64() / successful_reads as f64
        } else {
            0.0
        };
        let avg_bytes = if successful_reads > 0 {
            total_bytes as f64 / successful_reads as f64
        } else {
            0.0
        };

        println!("\nRead Performance Test Results:");
        println!("Total time: {:?}", total_time);
        println!("Average time per read: {:.2}ms", avg_time * 1000.0);
        println!("Total bytes read: {}", total_bytes);
        println!("Average bytes per read: {:.2}", avg_bytes);
        println!("Successful reads: {}", successful_reads);
    }

    #[test]
    fn test_nonblocking_read_performance() {
        // Initialize PTY with default arguments
        let args = PTYArgs { cols: 80, rows: 24, ..Default::default() };

        let mut pty = PTY::new_with_backend(&args, PTYBackend::ConPTY).unwrap();
        pty.spawn(
            "cmd.exe".into(),
            Some("/c echo test".into()),
            None,
            None
        ).unwrap();

        pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();

        // Wait for process to start
        std::thread::sleep(std::time::Duration::from_millis(100));

        let start = Instant::now();
        let mut total_bytes = 0;
        let mut read_count = 0;
        let mut empty_reads = 0;

        // Read 100 times in non-blocking mode
        for _ in 0..100 {
            match pty.read(false) {
                Ok(data) => {
                    if data.is_empty() {
                        empty_reads += 1;
                    } else {
                        total_bytes += data.len();
                        read_count += 1;
                    }
                }
                Err(_) => break,
            }
        }

        let duration = start.elapsed();
        println!("Non-blocking read performance test:");
        println!("Total time: {:?}", duration);
        println!("Average time per read: {:?}ms", duration.as_secs_f64() * 1000.0 / (read_count + empty_reads) as f64);
        println!("Total bytes read: {}", total_bytes);
        println!("Successful reads: {}", read_count);
        println!("Empty reads: {}", empty_reads);
        println!("Average bytes per successful read: {}", total_bytes.checked_div(read_count).unwrap_or(0));
    }

    /// Reproducer for issue #133 (defect 3): on the async ConPTY path, a
    /// chunk that completes synchronously is never added to `total_written`,
    /// so a small write that fits in one 8192-byte chunk returns 0 regardless
    /// of payload size.
    ///
    /// Expected: pty.write returns the UTF-8 byte count of the payload.
    /// Actual (current code): returns 0.
    #[test]
    fn test_write_returns_byte_count_small_payload() {
        let args = PTYArgs { cols: 80, rows: 24, ..Default::default() };
        let mut pty = PTY::new_with_backend(&args, PTYBackend::ConPTY).unwrap();
        pty.spawn(OsString::from("c:\\windows\\system32\\cmd.exe"), None, None, None).unwrap();
        // Answer ConPTY's startup DA / CPR queries so it starts emitting
        // output normally. Return value here is not asserted — this write is
        // only priming the handshake.
        pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();
        sleep(Duration::from_millis(500));

        // Pure-ASCII, fits in one chunk. UTF-8 byte count == character count.
        let payload = OsString::from("echo hello world\r\n");
        let expected_bytes: u32 = 18;

        let written = pty.write(payload).unwrap();
        // ConPTY `AllocConsole` steals the test harness's stdout, so a
        // panic or `ok` message doesn't reach cargo test's log. Record the
        // observed count to a file so a human (or CI) can read it after.
        let _ = std::fs::write(
            std::env::temp_dir().join("winpty_rs_issue_133_small.txt"),
            format!("expected={expected_bytes} written={written}\n"),
        );
        assert_eq!(
            written, expected_bytes,
            "defect 3: async write returned {written} for an {expected_bytes}-byte payload",
        );
    }

    /// Reproducer for issue #133 (defects 1 + 3): a multi-chunk write
    /// underreports the written bytes. The last chunk's completion is
    /// collected from a stack-local copy of `write_overlapped` the kernel
    /// never updated, and sync-completing chunks are never counted at all.
    ///
    /// With BUFFER_SIZE = 8192, 30_000 bytes splits into 4 chunks:
    /// 8192 + 8192 + 8192 + 5424. Each chunk's completion is lost in a
    /// different way; current code returns 0 for the whole write.
    #[test]
    fn test_write_returns_byte_count_multi_chunk() {
        let args = PTYArgs { cols: 80, rows: 24, ..Default::default() };
        let mut pty = PTY::new_with_backend(&args, PTYBackend::ConPTY).unwrap();
        pty.spawn(OsString::from("c:\\windows\\system32\\cmd.exe"), None, None, None).unwrap();
        pty.write(OsString::from("\x1b[?1;0c\x1b[0;0R")).unwrap();
        sleep(Duration::from_millis(500));

        // Mirror the shape from the report: 30 lines of 1,000 characters,
        // each newline-terminated. A single 30,000-char blob with no newline
        // is a worse case than anything real - a cooked-mode console input
        // buffer can fill and stop draining the pipe, which stalls the write
        // for reasons unrelated to the byte-count bug under test.
        const LINES: u32 = 30;
        const LINE_LEN: u32 = 1_000;
        let line: String = std::iter::repeat_n('a', LINE_LEN as usize).collect();
        let mut payload = String::new();
        for _ in 0..LINES {
            payload.push_str(&line);
            payload.push('\r');
            payload.push('\n');
        }
        // Pure ASCII, so the UTF-8 byte count equals the char count:
        // 30 lines * (1000 chars + CRLF) = 30_060 bytes.
        let expected_bytes: u32 = LINES * (LINE_LEN + 2);
        assert_eq!(payload.len() as u32, expected_bytes, "payload built wrong");

        let written = pty.write(OsString::from(payload)).unwrap();
        let _ = std::fs::write(
            std::env::temp_dir().join("winpty_rs_issue_133_multi.txt"),
            format!("expected={expected_bytes} written={written}\n"),
        );
        assert_eq!(
            written, expected_bytes,
            "defects 1+3: multi-chunk async write returned {written} for a {expected_bytes}-byte payload",
        );
    }
}
