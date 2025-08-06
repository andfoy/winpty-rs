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
    CreateEventExW, WaitForSingleObjectEx, CREATE_EVENT_MANUAL_RESET, EVENT_ALL_ACCESS, INFINITE,
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
use std::time::Duration;

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
                    Error::from_win32().into()
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
                    Error::from_win32().into()
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
    let os_str = "\0".repeat(BUFFER_SIZE);
    let mut buf_vec: Vec<u8> = os_str.as_str().as_bytes().to_vec();
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
            let err = Error::from_win32();
            if let None = lp_overlapped {
                Error::from_win32().into()
            } else if err.code() != ERROR_IO_PENDING.into() {
                Error::from_win32().into()
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
                        Error::from_win32().into()
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

    let mut vec_buf: Vec<u16> = std::iter::repeat(0).take(buf_vec.len()).collect();

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
            let err: HRESULT = Error::from_win32().into();
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
            let err: HRESULT = Error::from_win32().into();
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
            let err: HRESULT = Error::from_win32().into();
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
    /// Channel used to send the process handle to the reading thread.
    reader_process_out: Sender<Option<LocalHandle>>,
    /// Atomic flag to signal that the reading process has the process handle.
    reader_ready: Arc<AtomicBool>,
    /// Channel used to receive a response from the reading thread.
    reader_out_rx: Receiver<Option<Result<OsString, OsString>>>,
    /// PTY process is async
    async_: bool,
    /// Writing OVERLAPPED struct for async operation
    write_overlapped: Option<OVERLAPPED>,
    /// Write mutex for concurrent access under async IO
    write_mutex: Arc<Mutex<bool>>,
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
        if !async_ {
            // Keep only the reading thread channel
            let (reader_out_tx, reader_out_rx) =
                unbounded::
                <Option<Result<OsString, OsString>>>();
            let (reader_alive_tx, reader_alive_rx) = unbounded::<bool>();
            let (reader_process_tx, reader_process_rx) = unbounded::<Option<LocalHandle>>();
            let spinlock_clone = Arc::clone(&thread_arc);
            let reader_ready = Arc::clone(&reader_arc);

            let reader_thread = thread::spawn(move || {
                let process_result = reader_process_rx.recv();
                if let Ok(Some(process)) = process_result {
                    reader_ready.store(true, Ordering::Release);
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
                    spinlock_clone.store(false, Ordering::Release);
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
                reader_process_out: reader_process_tx,
                reader_ready: reader_arc,
                reader_out_rx,
                async_,
                write_overlapped: None,
                write_mutex: Arc::new(Mutex::new(false)),
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
                if let Ok(Some(process)) = process_result {
                    reader_ready.store(true, Ordering::Release);
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
                    spinlock_clone.store(false, Ordering::Release);
                }

                drop(reader_process_rx);
                drop(reader_alive_rx);
                drop(reader_out_tx);
                drop(reader_process_2_tx);
            });

            let alive_reader_atomic = Arc::clone(&thread_arc);
            let alive_thread = thread::spawn(move || {
                if let Ok(handle) = reader_process_2_rx.recv() {
                    let _ = wait_for_exit(handle.into());
                    unsafe {
                        while alive_reader_atomic.load(Ordering::Acquire) {
                            let _ = CancelIoEx(Into::<HANDLE>::into(conout), None);
                        }
                        match cleanup_tx {
                            None => (),
                            Some(tx) => {
                                // alive_tx.send(false);
                                let _ = tx.send(true).unwrap_or(());
                            }
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
                reader_process_out: reader_process_tx,
                reader_ready: reader_arc,
                reader_out_rx,
                async_,
                write_overlapped: Some(write_overlapped),
                write_mutex: Arc::new(Mutex::new(false)),
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
        const BUFFER_SIZE: usize = 8192;
        let vec_buf: Vec<u16> = buf.encode_wide().collect();

        unsafe {
            let required_size = WideCharToMultiByte(
                CP_UTF8,
                0,
                &vec_buf[..],
                None,
                PCSTR(ptr::null_mut::<u8>()),
                None,
            );

            let mut bytes_buf: Vec<u8> = std::iter::repeat(0)
                .take((required_size) as usize)
                .collect();

            WideCharToMultiByte(
                CP_UTF8,
                0,
                &vec_buf[..],
                Some(&mut bytes_buf[..]),
                PCSTR(ptr::null_mut::<u8>()),
                None,
            );

            let mut total_written = 0u32;
            let mut bytes_written = MaybeUninit::<u32>::uninit();
            let bytes_ptr: *mut u32 = ptr::addr_of_mut!(*bytes_written.as_mut_ptr());
            let bytes_ref = Some(bytes_ptr);

            let c_mutex = Arc::clone(&self.write_mutex);
            let mut write_pending = c_mutex.lock().unwrap();

            // Write in chunks
            for chunk in bytes_buf.chunks(BUFFER_SIZE) {
                if self.async_ {
                    if *write_pending {
                        *write_pending = false;
                        if GetOverlappedResult(
                            Into::<HANDLE>::into(self.conin),
                            &mut self.write_overlapped.unwrap(),
                            bytes_ptr,
                            true,
                        )
                        .is_err()
                        {
                            let err: HRESULT = Error::from_win32().into();
                            let result_msg = err.message();
                            let string = OsString::from(result_msg);
                            return Err(string);
                        } else {
                            total_written += bytes_written.assume_init();
                        }
                    }

                    let write_result = if WriteFile(
                        Into::<HANDLE>::into(self.conin),
                        Some(chunk),
                        bytes_ref,
                        Some(&mut self.write_overlapped.unwrap()),
                    )
                    .is_ok()
                    {
                        S_OK
                    } else {
                        let err = Error::from_win32();
                        if err.code() == ERROR_IO_PENDING.into() {
                            *write_pending = true;
                            S_OK
                        } else {
                            Error::from_win32().into()
                        }
                    };

                    if write_result.is_err() {
                        let result_msg = write_result.message();
                        let string = OsString::from(result_msg);
                        return Err(string);
                    }
                } else {
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
                        Error::from_win32().into()
                    };
                    if write_result.is_err() {
                        let result_msg = write_result.message();
                        let string = OsString::from(result_msg);
                        return Err(string);
                    }
                    total_written += bytes_written.assume_init();
                }
            }
            Ok(total_written)
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
                let result: HRESULT = Error::from_win32().into();
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

            while self.reader_atomic.load(Ordering::Acquire) {
                // Cancel all pending IO operations on conout
                let _ = CancelIoEx(Into::<HANDLE>::into(self.conout), None);

                // Send instruction to thread to finish
                if self.reader_alive.send(false).is_ok() {}
            }

            // Wait for the thread to be down
            if let Some(thread_handle) = self.reading_thread.take() {
                thread_handle.join().unwrap();
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

            if let Some(thread_handle) = self.alive_thread.take() {
                thread_handle.join().unwrap_or(());
            }
        }
    }
}
