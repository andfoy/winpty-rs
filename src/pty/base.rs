use windows::core::{Error, HRESULT, PCSTR};
/// Base struct used to generalize some of the PTY I/O operations.
use windows::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, HANDLE, STATUS_PENDING, S_OK, WAIT_FAILED, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Globalization::{
    MultiByteToWideChar, WideCharToMultiByte, CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS,
};
use windows::Win32::Storage::FileSystem::{GetFileSizeEx, ReadFile, WriteFile};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::Threading::{
    CreateEventExW, SetEvent, WaitForSingleObjectEx, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS,
    INFINITE,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, GetProcessId, WaitForSingleObject};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

use core::ffi::c_void;
use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long teardown will wait for the reader thread to signal that it has
/// exited before giving up and continuing anyway.
///
/// The reader signals only once its pending `ReadFile` returns, and that
/// relies on ConPTY closing the output pipe when the child exits. That is an
/// assumption about an external component whose behaviour we do not control -
/// a ConPTY update that changes it must not be able to deadlock us.
const READER_EXIT_BUDGET: Duration = Duration::from_millis(2_000);

/// Poll granularity while waiting for `reader_exit_event`.
const READER_EXIT_POLL_MS: u32 = 50;


#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::prelude::*;
#[cfg(unix)]
use std::vec::IntoIter;

use crossbeam_channel::{unbounded, Sender, Receiver};

use super::PTYArgs;

#[cfg(unix)]
trait OsStrExt {
    fn from_wide(x: &[u16]) -> OsString;
    fn encode_wide(&self) -> IntoIter<u16>;
}

#[cfg(unix)]
impl OsStrExt for OsString {
    fn from_wide(_: &[u16]) -> OsString {
        return OsString::new();
    }

    fn encode_wide(&self) -> IntoIter<u16> {
        return Vec::<u16>::new().into_iter();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalHandle(pub *mut c_void);

impl LocalHandle {
    pub fn is_invalid(&self) -> bool {
        self.0 == -1 as _ || self.0 == 0 as _
    }
}

unsafe impl Send for LocalHandle {}
unsafe impl Sync for LocalHandle {}

impl From<HANDLE> for LocalHandle {
    fn from(value: HANDLE) -> Self {
        Self(value.0)
    }
}

impl From<LocalHandle> for HANDLE {
    fn from(value: LocalHandle) -> Self {
        Self(value.0)
    }
}

/// This trait should be implemented by any backend that wants to provide a PTY implementation.
pub trait PTYImpl: Sync + Send {
    /// Create a new instance of the PTY backend.
    ///
    /// # Arguments
    /// * `args` - Arguments used to initialize the backend struct.
    ///
    /// # Returns
    /// * `pty`: The instantiated PTY struct.
    #[allow(clippy::new_ret_no_self)]
    fn new(args: &PTYArgs) -> Result<Box<dyn PTYImpl>, OsString>
    where
        Self: Sized;

    /// Spawn a process inside the PTY.
    ///
    /// # Arguments
    /// * `appname` - Full path to the executable binary to spawn.
    /// * `cmdline` - Optional space-delimited arguments to provide to the executable.
    /// * `cwd` - Optional path from where the executable should be spawned.
    /// * `env` - Optional environment variables to provide to the process. Each
    /// variable should be declared as `VAR=VALUE` and be separated by a NUL (0) character.
    ///
    /// # Returns
    /// `true` if the call was successful, else an error will be returned.
    fn spawn(
        &mut self,
        appname: OsString,
        cmdline: Option<OsString>,
        cwd: Option<OsString>,
        env: Option<OsString>,
    ) -> Result<bool, OsString>;

    /// Change the PTY size.
    ///
    /// # Arguments
    /// * `cols` - Number of character columns to display.
    /// * `rows` - Number of line rows to display.
    fn set_size(&self, cols: i32, rows: i32) -> Result<(), OsString>;

    /// Read from the process standard output.
    ///
    /// # Arguments
    /// * `blocking` - If true, wait for data to be available. If false, return immediately if no data is available.
    ///
    /// # Returns
    /// * `Ok(OsString)` - The data read from the process output
    /// * `Err(OsString)` - If EOF is reached or an error occurs
    ///
    /// # Notes
    /// * The actual read operation happens in a background thread with a fixed buffer size
    /// * The returned data is represented using a [`OsString`] since Windows operates over `u16` strings
    fn read(&self, blocking: bool) -> Result<OsString, OsString>;

    /// Write a (possibly) UTF-16 string into the standard input of a process.
    ///
    /// # Arguments
    /// * `buf` - [`OsString`] containing the string to write.
    ///
    /// # Returns
    /// The total number of characters written if the call was successful, else
    /// an [`OsString`] containing an human-readable error.
    fn write(&self, buf: OsString) -> Result<u32, OsString>;

    /// Check if a process reached End-of-File (EOF).
    ///
    /// # Returns
    /// `true` if the process reached EOL, false otherwise. If an error occurs, then a [`OsString`]
    /// containing a human-readable error is raised.
    fn is_eof(&self) -> Result<bool, OsString>;

    /// Retrieve the exit status of the process
    ///
    /// # Returns
    /// `None` if the process has not exited, else the exit code of the process.
    fn get_exitstatus(&self) -> Result<Option<u32>, OsString>;

    /// Determine if the process is still alive.
    fn is_alive(&self) -> Result<bool, OsString>;

    /// Retrieve the Process ID associated to the current process.
    fn get_pid(&self) -> u32;

    /// Retrieve the process handle ID of the spawned program.
    fn get_fd(&self) -> isize;

    /// Wait for the process to exit/finish.
    fn wait_for_exit(&self) -> Result<bool, OsString>;

    /// Cancel all pending I/O read operations.
    fn cancel_io(&self) -> Result<bool, OsString>;
}

fn read(
    blocking: bool,
    stream: HANDLE,
    using_pipes: bool,
    lp_overlapped: Option<*mut OVERLAPPED>,
) -> Result<(OsString, bool), OsString> {
    let mut result: HRESULT;
    if !blocking {
        if using_pipes {
            let mut bytes_u = MaybeUninit::<u32>::uninit();

            unsafe {
                let bytes_ptr = ptr::addr_of_mut!(*bytes_u.as_mut_ptr());
                let bytes_ref = bytes_ptr.as_mut().unwrap();

                result = if PeekNamedPipe(stream, None, 0, Some(bytes_ref), None, None).is_ok() {
                    S_OK
                } else {
                    Error::from_thread().into()
                };

                if result.is_err() {
                    let result_msg = result.message();
                    let string = OsString::from(result_msg);
                    return Err(string);
                }
            }
        } else {
            let mut size = MaybeUninit::<i64>::uninit();
            unsafe {
                let size_ptr = ptr::addr_of_mut!(*size.as_mut_ptr());
                let size_ref = size_ptr.as_mut().unwrap();
                result = if GetFileSizeEx(stream, size_ref).is_ok() {
                    S_OK
                } else {
                    Error::from_thread().into()
                };

                if result.is_err() {
                    let result_msg = result.message();
                    let string = OsString::from(result_msg);
                    return Err(string);
                }
                size.assume_init();
            }
        }
    }

    const BUFFER_SIZE: usize = 32768;
    let mut buf_vec: Vec<u8> = vec![0u8; BUFFER_SIZE];
    let mut chars_read = MaybeUninit::<u32>::uninit();
    let mut awaiting_io = false;
    unsafe {
        let chars_read_ptr = ptr::addr_of_mut!(*chars_read.as_mut_ptr());
        let chars_read_mut = Some(chars_read_ptr);
        result = if ReadFile(
            stream,
            Some(&mut buf_vec[..]),
            chars_read_mut,
            lp_overlapped,
        )
        .is_ok()
        {
            S_OK
        } else {
            let err = Error::from_thread();
            if let None = lp_overlapped {
                Error::from_thread().into()
            } else if err.code() != ERROR_IO_PENDING.into() {
                Error::from_thread().into()
            } else {
                awaiting_io = true;
                S_OK
            }
        };

        if result.is_err() {
            let result_msg = result.message();
            let string = OsString::from(result_msg);
            return Err(string);
        }

        if let Some(overlapped) = lp_overlapped {
            result = if awaiting_io {
                // awaiting_io = false;
                if (*overlapped).Internal == STATUS_PENDING.0 as usize {
                    if WaitForSingleObjectEx((*overlapped).hEvent, INFINITE, false) != WAIT_OBJECT_0
                    {
                        Error::from_thread().into()
                    } else {
                        *chars_read_ptr = (*overlapped).InternalHigh as u32;
                        HRESULT((*overlapped).Internal as i32).into()
                    }
                } else {
                    *chars_read_ptr = (*overlapped).InternalHigh as u32;
                    HRESULT((*overlapped).Internal as i32).into()
                }
            } else {
                S_OK
            };

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let read_bytes = chars_read.assume_init();
            if read_bytes == 0 {
                return Ok((OsString::new(), false));
            }
        }
    }

    // if let Some(true) = awaiting_io {
    //     return Ok((OsString::new(), awaiting_io));
    // }

    let mut vec_buf: Vec<u16> = vec![0u16; buf_vec.len()];

    unsafe {
        MultiByteToWideChar(
            CP_UTF8,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            &buf_vec[..],
            Some(&mut vec_buf[..]),
        );
    }

    let non_zeros_init = Vec::new();
    let non_zeros: Vec<u16> =
        vec_buf
            .split(|x| x == &0)
            .map(|x| x.to_vec())
            .fold(non_zeros_init, |mut acc, mut x| {
                acc.append(&mut x);
                acc
            });
    let os_str = OsString::from_wide(&non_zeros[..]);
    Ok((os_str, true))
}

fn is_alive(process: HANDLE) -> Result<bool, OsString> {
    unsafe {
        let is_timeout = WaitForSingleObject(process, 0);
        let succ = is_timeout != WAIT_FAILED;

        if succ {
            let alive = is_timeout == WAIT_TIMEOUT;
            Ok(alive)
        } else {
            let err: HRESULT = Error::from_thread().into();
            let result_msg = err.message();
            let string = OsString::from(result_msg);
            Err(string)
        }
    }
}

fn wait_for_exit(process: HANDLE) -> Result<bool, OsString> {
    unsafe {
        let wait_status = WaitForSingleObject(process, INFINITE);
        let succ = wait_status != WAIT_FAILED;
        if succ {
            let dead = wait_status == WAIT_OBJECT_0;
            Ok(dead)
        } else {
            let err: HRESULT = Error::from_thread().into();
            let result_msg = err.message();
            let string = OsString::from(result_msg);
            Err(string)
        }
    }
}

fn get_exitstatus(process: HANDLE) -> Result<Option<u32>, OsString> {
    let mut exit = MaybeUninit::<u32>::uninit();
    unsafe {
        let exit_ptr: *mut u32 = ptr::addr_of_mut!(*exit.as_mut_ptr());
        let exit_ref = exit_ptr.as_mut().unwrap();
        let succ = GetExitCodeProcess(process, exit_ref).is_ok();

        if succ {
            let actual_exit = *exit_ptr;
            exit.assume_init();
            let alive = actual_exit == STATUS_PENDING.0 as u32;
            let mut exitstatus: Option<u32> = None;
            if !alive {
                exitstatus = Some(actual_exit);
            }
            Ok(exitstatus)
        } else {
            let err: HRESULT = Error::from_thread().into();
            let result_msg = err.message();
            let string = OsString::from(result_msg);
            Err(string)
        }
    }
}

fn is_eof(process: HANDLE, stream: HANDLE) -> Result<bool, OsString> {
    let mut bytes = MaybeUninit::<u32>::uninit();
    unsafe {
        let bytes_ptr: *mut u32 = ptr::addr_of_mut!(*bytes.as_mut_ptr());
        let bytes_ref = Some(bytes_ptr);
        let succ = PeekNamedPipe(stream, None, 0, None, bytes_ref, None).is_ok();

        let total_bytes = bytes.assume_init();
        if succ {
            match is_alive(process) {
                Ok(alive) => {
                    let eof = !alive && total_bytes == 0;
                    Ok(eof)
                }
                Err(_) => Ok(true),
            }
        } else {
            Ok(true)
        }
    }
}

/// This struct handles the I/O operations to the standard streams, as well
/// the lifetime of a process running inside a PTY.
pub struct PTYProcess {
    /// Handle to the process to read from.
    process: LocalHandle,
    /// Handle to the standard input stream.
    conin: LocalHandle,
    /// Handle to the standard output stream.
    conout: LocalHandle,
    /// Identifier of the process running inside the PTY.
    pid: u32,
    /// Close process when the struct is dropped.
    close_process: bool,
    /// Handle to the thread used to read from the standard output.
    reading_thread: Option<thread::JoinHandle<()>>,
    /// Handle to the thread used to check if the process is alive.
    alive_thread: Option<thread::JoinHandle<()>>,
    /// Channel used to keep the thread alive.
    reader_alive: Sender<bool>,
    /// Atomic variable to signal when a thread finishes
    reader_atomic: Arc<AtomicBool>,
    /// Manual-reset event signaled by the reader thread on exit; lets teardown wait
    /// event-driven instead of busy-looping on `reader_atomic`.
    reader_exit_event: LocalHandle,
    /// Channel used to send the process handle to the reading thread.
    reader_process_out: Sender<Option<LocalHandle>>,
    /// Atomic flag to signal that the reading process has the process handle.
    reader_ready: Arc<AtomicBool>,
    /// Channel used to receive a response from the reading thread.
    reader_out_rx: Receiver<Option<Result<OsString, OsString>>>,
    /// PTY process is async
    async_: bool,
    /// Serialised write state used by the async ConPTY path. The `OVERLAPPED`
    /// struct, its event, and the owned source buffer all live inside the
    /// mutex so their addresses stay stable for the kernel's duration — the
    /// previous code passed `.unwrap()`ed stack copies of the OVERLAPPED to
    /// `WriteFile`/`GetOverlappedResult` and leaked the chunk slice back to
    /// the caller before the kernel finished with it (#133).
    write_mutex: Mutex<WriteState>,
}

/// Writer state owned by `PTYProcess` when the ConPTY async path is active.
///
/// The three fields are kept together so that holding `write_mutex` grants
/// exclusive access to the OVERLAPPED struct, its backing event, and the
/// source buffer currently being handed to the kernel. Each has a stable
/// address for the lifetime of the mutex guard, which is what makes
/// overlapped I/O sound here.
struct WriteState {
    /// `true` while a previously-submitted `WriteFile` is still in flight.
    /// Normally cleared before `write()` returns; the first thing a new call
    /// does is drain any carry-over left behind by an interrupted prior
    /// call (defensive).
    pending: bool,
    /// OVERLAPPED struct re-used across writes. Its `hEvent` is a
    /// manual-reset event created once on construction and freed in
    /// `Drop for PTYProcess`.
    overlapped: OVERLAPPED,
    /// Owns the UTF-8-encoded bytes currently handed to `WriteFile`. Kept
    /// alive through the pending window so the kernel never reads freed
    /// memory.
    buffer: Vec<u8>,
}

// `OVERLAPPED.hEvent` is a raw `HANDLE` (a pointer). The default auto-impl
// of Send/Sync for a Mutex<WriteState> would otherwise block cross-thread
// access to the lock. All kernel-visible writes to this field happen through
// Win32 overlapped I/O which is safe to issue from any thread as long as the
// handle isn't aliased, and the mutex guarantees that.
unsafe impl Send for WriteState {}

/// How long a single overlapped write may stay pending before we give up,
/// cancel it and report an error.
///
/// This must be bounded. The ConPTY input pipe can remain backed up for an
/// unbounded time when nothing is draining it - most visibly when a caller
/// writes immediately after `spawn` and the child has not started reading
/// its stdin yet. An unbounded wait there deadlocks the caller permanently
/// (it hung the `src/pty.rs:115` doc-test in CI, which does exactly that).
const WRITE_COMPLETION_TIMEOUT_MS: u32 = 10_000;

/// Collect the result of a pending overlapped operation without waiting
/// forever.
///
/// Waits up to `timeout_ms` on the operation's event. If it completes, the
/// transferred byte count is returned. On timeout the I/O is cancelled with
/// `CancelIoEx` and we then wait (briefly, and unconditionally) for the
/// cancellation to land, because the caller's source buffer may not be
/// released or reused until the kernel has genuinely finished with it.
///
/// # Safety
/// `overlapped` must point at the exact OVERLAPPED struct previously handed
/// to the pending `ReadFile`/`WriteFile` on `handle`, and must stay valid
/// for the duration of this call.
unsafe fn drain_overlapped(
    handle: HANDLE,
    overlapped: *mut OVERLAPPED,
    timeout_ms: u32,
) -> Result<u32, OsString> {
    let event = (*overlapped).hEvent;
    let mut transferred: u32 = 0;

    // With a non-null hEvent the kernel resets the event on submission and
    // signals it on completion, so waiting on it is well-defined here.
    let waited = if event.is_invalid() {
        // No event to wait on; fall back to polling the status field.
        WAIT_OBJECT_0
    } else {
        WaitForSingleObject(event, timeout_ms)
    };

    if waited == WAIT_OBJECT_0 {
        if GetOverlappedResult(handle, overlapped, &mut transferred as *mut u32, false).is_ok() {
            return Ok(transferred);
        }
        let err: HRESULT = Error::from_thread().into();
        return Err(OsString::from(err.message()));
    }

    // Timed out (or the wait itself failed). Cancel so the kernel stops
    // touching the source buffer, then reap the cancellation. The second
    // wait uses `true` because by this point the operation is either
    // complete or cancelled, so it cannot block indefinitely.
    let _ = CancelIoEx(handle, Some(overlapped));
    let _ = GetOverlappedResult(handle, overlapped, &mut transferred as *mut u32, true);

    Err(OsString::from(format!(
        "Timed out after {timeout_ms}ms waiting for a pending PTY write to complete; \
         the operation was cancelled after {transferred} byte(s) were transferred"
    )))
}

impl PTYProcess {
    /// Create a new [`PTYProcess`] instance.
    ///
    /// # Arguments
    /// * `conin` - Handle to the process standard input stream
    /// * `conout` - Handle to the process standard output stream
    /// * `using_pipes` - `true` if the streams are Windows named pipes, `false` if they are files.
    /// * `async_` - `true` if the streams are async, `false` if they are sync.
    ///
    /// # Returns
    /// * `pty` - A new [`PTYProcess`] instance.
    pub fn new(
        conin: LocalHandle,
        conout: LocalHandle,
        using_pipes: bool,
        async_: bool,
        cleanup_tx: Option<mpsc::Sender<bool>>,
    ) -> PTYProcess {
        let thread_arc = Arc::new(AtomicBool::new(true));
        let reader_arc = Arc::new(AtomicBool::new(false));

        // Manual-reset event signaled by the reader thread when it exits. Teardown
        // waits on this (event-driven) instead of busy-polling `thread_arc`.
        let reader_exit_handle = unsafe {
            match CreateEventExW(None, None, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS.0) {
                Ok(evt) => evt,
                Err(_) => HANDLE::default(),
            }
        };
        let reader_exit_event = LocalHandle::from(reader_exit_handle);

        if !async_ {
            // Keep only the reading thread channel
            let (reader_out_tx, reader_out_rx) =
                unbounded::
                <Option<Result<OsString, OsString>>>();
            let (reader_alive_tx, reader_alive_rx) = unbounded::<bool>();
            let (reader_process_tx, reader_process_rx) = unbounded::<Option<LocalHandle>>();
            let spinlock_clone = Arc::clone(&thread_arc);
            let reader_ready = Arc::clone(&reader_arc);
            let reader_exit_for_thread = reader_exit_event;

            let reader_thread = thread::spawn(move || {
                let process_result = reader_process_rx.recv();
                reader_ready.store(true, Ordering::Release);
                if let Ok(Some(process)) = process_result {
                    let mut alive = reader_alive_rx
                        .try_recv()
                        .unwrap_or(true);
                    while alive
                    {
                        if !is_eof(process.into(), conout.into()).unwrap() {
                            match read(true, conout.into(), using_pipes, None) {
                                Ok((result, _)) => {
                                    reader_out_tx.send(Some(Ok(result))).unwrap();
                                }
                                Err(err) => {
                                    reader_out_tx.send(Some(Err(err))).unwrap();
                                }
                            }
                            alive = reader_alive_rx
                        .try_recv()
                        .unwrap_or(true);
                        } else {
                            reader_out_tx.send(None).unwrap();
                            alive = false;
                        }
                    }
                }
                spinlock_clone.store(false, Ordering::Release);

                unsafe {
                    let _ = SetEvent(Into::<HANDLE>::into(reader_exit_for_thread));
                }

                drop(reader_process_rx);
                drop(reader_alive_rx);
                drop(reader_out_tx);
            });

            PTYProcess {
                process: LocalHandle(std::ptr::null_mut()),
                conin,
                conout,
                pid: 0,
                close_process: true,
                reading_thread: Some(reader_thread),
                alive_thread: None,
                reader_alive: reader_alive_tx,
                reader_atomic: thread_arc,
                reader_exit_event,
                reader_process_out: reader_process_tx,
                reader_ready: reader_arc,
                reader_out_rx,
                async_,
                write_mutex: Mutex::new(WriteState {
                    pending: false,
                    overlapped: OVERLAPPED::default(),
                    buffer: Vec::new(),
                }),
            }
        } else {
            let mut write_overlapped = OVERLAPPED::default();
            unsafe {
                match CreateEventExW(None, None, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS.0) {
                    Ok(evt) => {
                        write_overlapped.hEvent = evt;
                    }

                    Err(_) => (),
                }
            }

            let (reader_out_tx, reader_out_rx) =
                unbounded::<Option<Result<OsString, OsString>>>();
            let (reader_alive_tx, reader_alive_rx) = unbounded::<bool>();
            let (reader_process_tx, reader_process_rx) = unbounded::<Option<LocalHandle>>();
            let spinlock_clone = Arc::clone(&thread_arc);
            let reader_ready = Arc::clone(&reader_arc);
            let (reader_process_2_tx, reader_process_2_rx) = unbounded::<LocalHandle>();
            let reader_exit_for_thread = reader_exit_event;
            let reader_exit_for_alive = reader_exit_event;

            let reader_thread = thread::spawn(move || {
                let mut read_overlapped = OVERLAPPED::default();
                unsafe {
                    match CreateEventExW(None, None, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS.0)
                    {
                        Ok(evt) => {
                            read_overlapped.hEvent = evt;
                        }

                        Err(_) => (),
                    }
                }

                let process_result = reader_process_rx.recv();
                reader_ready.store(true, Ordering::Release);
                if let Ok(Some(process)) = process_result {
                    let _ = reader_process_2_tx.send(process);
                    let mut alive = true;
                    while alive {
                        match read(true, conout.into(), using_pipes, Some(&mut read_overlapped)) {
                            Ok((result, alive_status)) => {
                                reader_out_tx.send(Some(Ok(result))).unwrap();
                                alive = alive_status;
                            }
                            Err(err) => {
                                reader_out_tx.send(Some(Err(err))).unwrap();
                                alive = false;
                            }
                        }
                    }

                    unsafe {
                        let _ = CloseHandle(read_overlapped.hEvent);
                    }
                }
                spinlock_clone.store(false, Ordering::Release);

                unsafe {
                    let _ = SetEvent(Into::<HANDLE>::into(reader_exit_for_thread));
                }

                drop(reader_process_rx);
                drop(reader_alive_rx);
                drop(reader_out_tx);
                drop(reader_process_2_tx);
            });

            let alive_thread = thread::spawn(move || {
                if let Ok(handle) = reader_process_2_rx.recv() {
                    // Unbounded on purpose. This thread lives for the whole
                    // lifetime of the PTY, not just teardown, so a timeout
                    // here would fire during normal operation and tear down a
                    // perfectly healthy session. Teardown is instead made
                    // safe by `Drop` not joining this thread - see there.
                    let _ = wait_for_exit(handle.into());
                    unsafe {
                        // Child has exited. Let modern ConPTY auto-close the output
                        // pipe — reader's pending ReadFile then returns 0 bytes
                        // (natural EOF) and the reader exits, signaling
                        // `reader_exit_event`. If that doesn't happen in time
                        // (older ConPTY / non-ConPTY consumer), fall back to
                        // CancelIoEx to unstick the reader.
                        let exit_handle = Into::<HANDLE>::into(reader_exit_for_alive);
                        if WaitForSingleObject(exit_handle, 5000) != WAIT_OBJECT_0 {
                            let _ = CancelIoEx(Into::<HANDLE>::into(conout), None);
                            // Bounded. This used to spin forever if the
                            // reader never signalled, which wedges this
                            // thread and, worse, withholds the cleanup
                            // signal below so the pseudoconsole is never
                            // released.
                            let deadline = Instant::now() + READER_EXIT_BUDGET;
                            loop {
                                if WaitForSingleObject(exit_handle, READER_EXIT_POLL_MS)
                                    == WAIT_OBJECT_0
                                {
                                    break;
                                }
                                if Instant::now() >= deadline {
                                    // Give up. The reader cannot be unstuck
                                    // from here, and falling through at least
                                    // releases the pseudoconsole rather than
                                    // leaking it along with this thread.
                                    break;
                                }
                                let _ = CancelIoEx(Into::<HANDLE>::into(conout), None);
                            }
                        }
                        // Reader has drained and exited. Signal cleanup so the
                        // consumer (e.g. ConPTY's cleanup_thread) can release
                        // resources via ClosePseudoConsole.
                        if let Some(tx) = cleanup_tx {
                            let _ = tx.send(true).unwrap_or(());
                        }
                    }
                }
                drop(reader_process_2_rx);
            });

            PTYProcess {
                process: LocalHandle(std::ptr::null_mut()),
                conin,
                conout,
                pid: 0,
                close_process: true,
                reading_thread: Some(reader_thread),
                alive_thread: Some(alive_thread),
                reader_alive: reader_alive_tx,
                reader_atomic: thread_arc,
                reader_exit_event,
                reader_process_out: reader_process_tx,
                reader_ready: reader_arc,
                reader_out_rx,
                async_,
                write_mutex: Mutex::new(WriteState {
                    pending: false,
                    overlapped: write_overlapped,
                    buffer: Vec::new(),
                }),
            }
        }
    }

    /// Read from the process standard output.
    ///
    /// # Arguments
    /// * `blocking` - If true, wait for data to be available. If false, return immediately if no data is available.
    ///
    /// # Returns
    /// * `Ok(OsString)` - The data read from the process output
    /// * `Err(OsString)` - If EOF is reached or an error occurs
    ///
    /// # Notes
    /// * The actual read operation happens in a background thread with a fixed buffer size
    /// * The returned data is represented using a [`OsString`] since Windows operates over `u16` strings
    pub fn read(&self, blocking: bool) -> Result<OsString, OsString> {
        // Get data directly from reading thread
        match blocking {
            true => match self.reader_out_rx.recv() {
                Ok(None) => Err(OsString::from("Standard out reached EOF")),
                Ok(Some(bytes)) => bytes,
                Err(_) => Ok(OsString::new()),
            },
            false => match self.reader_out_rx.try_recv() {
                Ok(None) => Err(OsString::from("Standard out reached EOF")),
                Ok(Some(bytes)) => bytes,
                Err(_) => Ok(OsString::new()),
            },
        }
    }

    /// Write an (possibly) UTF-16 string into the standard input of a process.
    ///
    /// # Arguments
    /// * `buf` - [`OsString`] containing the string to write.
    ///
    /// # Returns
    /// The total number of characters written if the call was successful, else
    /// an [`OsString`] containing an human-readable error.
    pub fn write(&self, buf: OsString) -> Result<u32, OsString> {
        let vec_buf: Vec<u16> = buf.encode_wide().collect();

        unsafe {
            // Convert the UTF-16 input to UTF-8. `WideCharToMultiByte` called
            // with `cbMultiByte = 0`/`lpMultiByteStr = null` reports the exact
            // byte count required (no trailing NUL since `vec_buf` has none).
            let required_size = WideCharToMultiByte(
                CP_UTF8,
                0,
                &vec_buf[..],
                None,
                PCSTR(ptr::null_mut::<u8>()),
                None,
            );

            if required_size == 0 {
                return Ok(0);
            }

            let mut bytes_buf: Vec<u8> = vec![0u8; required_size as usize];
            WideCharToMultiByte(
                CP_UTF8,
                0,
                &vec_buf[..],
                Some(&mut bytes_buf[..]),
                PCSTR(ptr::null_mut::<u8>()),
                None,
            );

            // Sync path (WinPTY and any other non-ConPTY backend, which
            // construct `PTYProcess` with `async_ = false`).
            //
            // Kept byte-for-byte as it was: chunked at 8 KiB, one blocking
            // `WriteFile` per chunk, accumulating the per-chunk counts.
            // #133 is exclusively about the async/ConPTY path, and a
            // synchronous `WriteFile` has no cancellation escape - it blocks
            // until the reader (winpty-agent) drains - so the chunking is
            // load-bearing rather than cosmetic: it bounds each write
            // against the pipe buffer instead of betting the whole payload
            // fits.
            if !self.async_ {
                const BUFFER_SIZE: usize = 8192;
                let mut total_written = 0u32;
                let mut bytes_written = MaybeUninit::<u32>::uninit();
                let bytes_ptr: *mut u32 = ptr::addr_of_mut!(*bytes_written.as_mut_ptr());
                let bytes_ref = Some(bytes_ptr);

                for chunk in bytes_buf.chunks(BUFFER_SIZE) {
                    let write_result = if WriteFile(
                        Into::<HANDLE>::into(self.conin),
                        Some(chunk),
                        bytes_ref,
                        None,
                    )
                    .is_ok()
                    {
                        S_OK
                    } else {
                        Error::from_thread().into()
                    };
                    if write_result.is_err() {
                        let result_msg = write_result.message();
                        let string = OsString::from(result_msg);
                        return Err(string);
                    }
                    total_written += bytes_written.assume_init();
                }
                return Ok(total_written);
            }

            // Async path (ConPTY). Shape mirrors Microsoft Terminal's
            // `ConptyConnection::WriteInput`: hold the writer lock for the
            // whole call, drain any carry-over pending write at the top (not
            // the bottom - see #133 defect 2), then issue one overlapped
            // `WriteFile` against the stable `state.overlapped` address with
            // the source bytes owned by `state.buffer` (not a stack slice
            // that drops on return - see #133 defect 1/2). Fixes:
            //   #133 defect 1: OVERLAPPED was passed as `&mut
            //     self.write_overlapped.unwrap()`, a stack-local copy; the
            //     kernel scribbled on freed memory on pend.
            //   #133 defect 2: a chunk's source slice pointed into a local
            //     Vec; the function returned with the write still pending
            //     and the Vec dropped, so the kernel read freed memory.
            //   #133 defect 3: sync-complete chunks never incremented
            //     `total_written`; small writes returned 0.
            let mut state = self.write_mutex.lock().map_err(|_| {
                OsString::from("writer mutex poisoned")
            })?;

            // Drain any leftover pending write first. Under normal flow
            // this branch does nothing (the previous call drained before
            // returning); it only runs if a prior call unwound before its
            // drain, in which case we must collect that completion before
            // reusing `state.overlapped`.
            if state.pending {
                // Bounded: never wait forever for a carry-over.
                let _ = drain_overlapped(
                    Into::<HANDLE>::into(self.conin),
                    &mut state.overlapped,
                    WRITE_COMPLETION_TIMEOUT_MS,
                );
                state.pending = false;
                // `state.buffer` can now be reused; either the kernel
                // finished with it or `drain_overlapped` cancelled the
                // operation and waited for the cancellation to land.
            }

            // Move the new payload into the member buffer so its address is
            // stable through the pending window. Terminal does the same with
            // its `_writeBuffer` member.
            state.buffer.clear();
            state.buffer.extend_from_slice(&bytes_buf);

            // Issue the write against the member OVERLAPPED/buffer. One call,
            // no chunking - ConPTY's input pipe accepts arbitrary sizes and
            // Terminal does not chunk either.
            let mut bytes_written: u32 = 0;
            let buf_ptr = state.buffer.as_ptr();
            let buf_len = state.buffer.len();
            // Build a slice tied to the member buffer's lifetime so WriteFile
            // sees the stable address. Using `state.buffer.as_slice()`
            // directly would re-borrow `state` while `state.overlapped` is
            // also borrowed below; splitting the raw pieces sidesteps that.
            let buf_slice = std::slice::from_raw_parts(buf_ptr, buf_len);
            let overlapped_ptr: *mut OVERLAPPED = &mut state.overlapped;

            let write_ok = WriteFile(
                Into::<HANDLE>::into(self.conin),
                Some(buf_slice),
                Some(&mut bytes_written as *mut u32),
                Some(overlapped_ptr),
            );

            match write_ok {
                Ok(_) => {
                    // Sync completion. The kernel has already read the
                    // buffer; nothing is pending.
                    Ok(bytes_written)
                }
                Err(err) if err.code() == ERROR_IO_PENDING.into() => {
                    // Overlapped submission accepted but not yet complete.
                    // We must not return while the kernel still owns
                    // `state.buffer`, and we must not wait forever: the
                    // ConPTY input pipe can stay backed up indefinitely if
                    // nothing is draining it (e.g. a freshly-spawned child
                    // that has not started reading stdin). An unbounded
                    // `GetOverlappedResult(.., true)` here deadlocks the
                    // caller - it hung the doc-test at src/pty.rs:115 in CI,
                    // which writes immediately after spawn with no reader.
                    state.pending = true;
                    let drained = drain_overlapped(
                        Into::<HANDLE>::into(self.conin),
                        &mut state.overlapped,
                        WRITE_COMPLETION_TIMEOUT_MS,
                    );
                    state.pending = false;
                    match drained {
                        Ok(n) => Ok(n),
                        Err(msg) => Err(msg),
                    }
                }
                Err(err) => Err(OsString::from(err.message())),
            }
        }
    }

    /// Check if a process reached End-of-File (EOF).
    ///
    /// # Returns
    /// `true` if the process reached EOL, false otherwise. If an error occurs, then a [`OsString`]
    /// containing a human-readable error is raised.
    pub fn is_eof(&self) -> Result<bool, OsString> {
        // let mut available_bytes: Box<u32> = Box::new_uninit();
        // let bytes_ptr: *mut u32 = &mut *available_bytes;
        // let bytes_ptr: *mut u32 = ptr::null_mut();
        let mut bytes = MaybeUninit::<u32>::uninit();
        unsafe {
            let bytes_ptr: *mut u32 = ptr::addr_of_mut!(*bytes.as_mut_ptr());
            let bytes_ref = Some(bytes_ptr);
            let mut succ = PeekNamedPipe(
                Into::<HANDLE>::into(self.conout),
                None,
                0,
                bytes_ref,
                None,
                None,
            )
            .is_ok();

            let _total_bytes = bytes.assume_init();

            let is_alive = match self.is_alive() {
                Ok(alive) => {
                    alive || !self.reader_out_rx.is_empty()
                },
                Err(err) => {
                    return Err(err);
                }
            };

            succ = succ || is_alive || self.reader_atomic.load(Ordering::Acquire);
            Ok(!succ)
        }
    }

    /// Retrieve the exit status of the process
    ///
    /// # Returns
    /// `None` if the process has not exited, else the exit code of the process.
    pub fn get_exitstatus(&self) -> Result<Option<u32>, OsString> {
        if self.pid == 0 {
            return Ok(None);
        }

        match get_exitstatus(self.process.into()) {
            Ok(exitstatus) => Ok(exitstatus),
            Err(err) => Err(err),
        }
    }

    /// Determine if the process is still alive.
    pub fn is_alive(&self) -> Result<bool, OsString> {
        // let mut exit_code: Box<u32> = Box::new_uninit();
        // let exit_ptr: *mut u32 = &mut *exit_code;
        match is_alive(self.process.into()) {
            Ok(alive) => Ok(alive),
            Err(err) => Err(err),
        }
    }

    /// Set the running process behind the PTY.
    pub fn set_process(&mut self, process: HANDLE, close_process: bool) {
        self.process = process.into();
        self.close_process = close_process;

        // if env::var_os("CONPTY_CI").is_some() {
        //     // For some reason, the CI requires a flush of the handle before
        //     // reading from a thread.
        //     let result = read(4096, true, self.conout, false).unwrap();
        //     println!("{:?}", result);
        //     let result = read(4096, true, self.conout, false).unwrap();
        //     println!("{:?}", result);
        //     let res: Result<u32, OsString> = self.write(OsString::from("\r\n\r\n"));
        //     res.unwrap();
        // }

        self.reader_process_out.send(Some(process.into())).unwrap();
        unsafe {
            self.pid = GetProcessId(Into::<HANDLE>::into(self.process));
        }
    }

    /// Retrieve the Process ID associated to the current process.
    pub fn get_pid(&self) -> u32 {
        self.pid
    }

    /// Retrieve the process handle ID of the spawned program.
    pub fn get_fd(&self) -> isize {
        self.process.0 as isize
    }

    /// Wait for the process to exit
    pub fn wait_for_exit(&self) -> Result<bool, OsString> {
        wait_for_exit(self.process.into())
    }

    /// Cancel all pending I/O operations
    pub fn cancel_io(&self) -> Result<bool, OsString> {
        unsafe {
            if CancelIoEx(Into::<HANDLE>::into(self.conout), None).is_ok() {
                Ok(true)
            } else {
                let result: HRESULT = Error::from_thread().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                Err(string)
            }
        }
    }
}

impl Drop for PTYProcess {
    fn drop(&mut self) {
        unsafe {
            while !self.reader_ready.load(Ordering::Acquire) {
                // Unblock thread if it is waiting for a process handle.
                if self.reader_process_out.send(None).is_ok() {}
            }

            // Signal the sync reader (which polls the channel) that it should exit,
            // and cancel any pending I/O so a blocked ReadFile returns. Then wait
            // event-driven for the reader to actually exit. Retry on timeout to
            // handle the race where the reader issues a new read between cancel
            // and wait.
            let _ = self.reader_alive.send(false);
            let _ = CancelIoEx(Into::<HANDLE>::into(self.conout), None);

            // Bounded wait for the reader to acknowledge it has exited.
            //
            // This was an unbounded `loop`, which deadlocks `drop` outright
            // whenever the reader never signals. A wedged reader thread is
            // survivable - process teardown reaps it - but a wedged `drop`
            // is not: it hangs whoever dropped the PTY. Through pywinpty that
            // means hanging Jupyter, so this must always make progress.
            let exit_handle = Into::<HANDLE>::into(self.reader_exit_event);
            let mut reader_exited = true;
            if !self.reader_exit_event.is_invalid() {
                reader_exited = false;
                let deadline = Instant::now() + READER_EXIT_BUDGET;
                loop {
                    if WaitForSingleObject(exit_handle, READER_EXIT_POLL_MS) == WAIT_OBJECT_0 {
                        reader_exited = true;
                        break;
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                    let _ = self.reader_alive.send(false);
                    let _ = CancelIoEx(Into::<HANDLE>::into(self.conout), None);
                }
            }

            // Only join a reader we know has finished. Joining one still
            // blocked in `ReadFile` would reintroduce exactly the deadlock
            // bounded above. The result is deliberately discarded rather than
            // unwrapped: panicking inside `drop` risks a double panic and an
            // abort.
            if let Some(thread_handle) = self.reading_thread.take() {
                if reader_exited {
                    let _ = thread_handle.join();
                }
            }

            if !self.conin.is_invalid() {
                let _ = CloseHandle(Into::<HANDLE>::into(self.conin));
            }

            if !self.conout.is_invalid() && !self.async_ {
                let _ = CloseHandle(Into::<HANDLE>::into(self.conout));
            }

            if self.close_process && !self.process.is_invalid() {
                let _ = CloseHandle(Into::<HANDLE>::into(self.process));
            }

            // Detach `alive_thread` rather than joining it.
            //
            // It blocks in `wait_for_exit(child)` for the PTY's whole
            // lifetime, so joining here deadlocks whenever the child outlives
            // `ClosePseudoConsole` - which ConPTY 1.25 no longer guarantees
            // as promptly as 1.24 did. That is what hung teardown, orphaned
            // the child process and wedged the doc-tests in CI.
            //
            // Detaching is sound: `ConPTY::drop` has already called
            // `ClosePseudoConsole` by the time we get here (the outer drop
            // body runs before its fields are dropped), so releasing the
            // pseudoconsole does not depend on this thread. Dropping the
            // `JoinHandle` detaches it and process teardown reaps it.
            drop(self.alive_thread.take());

            if !self.reader_exit_event.is_invalid() {
                let _ = CloseHandle(exit_handle);
            }

            // Close the writer's OVERLAPPED event. Previously this was never
            // freed (leaked once per PTY on the async path).
            //
            // Deliberately *not* taking `write_mutex` here: `drop` receives
            // `&mut self`, so we already have exclusive access, and taking a
            // lock during teardown risks wedging it behind an in-flight
            // write. `get_mut` asserts that statically instead.
            if let Ok(state) = self.write_mutex.get_mut() {
                let write_event = state.overlapped.hEvent;
                if !write_event.is_invalid() {
                    let _ = CloseHandle(write_event);
                }
            }
        }
    }
}
