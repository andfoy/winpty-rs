/// Base struct used to generalize some of the PTY I/O operations.

use windows::Win32::Foundation::{CloseHandle, HANDLE, STATUS_PENDING, S_OK, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{GetFileSizeEx, ReadFile, WriteFile};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Threading::{GetExitCodeProcess, GetProcessId, WaitForSingleObject};
use windows::Win32::Globalization::{MultiByteToWideChar, WideCharToMultiByte, CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::core::{HRESULT, Error, PCSTR};
use windows::Win32::System::Threading::INFINITE;

use std::ptr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::mem::MaybeUninit;
use std::cmp::min;
use std::ffi::OsString;
use core::ffi::c_void;

#[cfg(windows)]
use std::os::windows::prelude::*;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(unix)]
use std::vec::IntoIter;

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
        where Self: Sized;

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
    fn spawn(&mut self, appname: OsString, cmdline: Option<OsString>, cwd: Option<OsString>, env: Option<OsString>) -> Result<bool, OsString>;

    /// Change the PTY size.
    ///
    /// # Arguments
    /// * `cols` - Number of character columns to display.
    /// * `rows` - Number of line rows to display.
    fn set_size(&self, cols: i32, rows: i32) -> Result<(), OsString>;

    /// Read at most `length` characters from a process standard output.
    ///
    /// # Arguments
    /// * `length` - Upper limit on the number of characters to read.
    /// * `blocking` - Block the reading thread if no bytes are available.
    ///
    /// # Notes
    /// * If `blocking = false`, then the function will check how much characters are available on
    /// the stream and will read the minimum between the input argument and the total number of
    /// characters available.
    ///
    /// * The bytes returned are represented using a [`OsString`] since Windows operates over
    /// `u16` strings.
    fn read(&self, length: u32, blocking: bool) -> Result<OsString, OsString>;

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
}


fn read(mut length: u32, blocking: bool, stream: HANDLE, using_pipes: bool) -> Result<OsString, OsString> {
    let mut result: HRESULT;
    if !blocking {
        if using_pipes {
            let mut bytes_u = MaybeUninit::<u32>::uninit();

            //let mut available_bytes = Box::<>::new_uninit();
            //let bytes_ptr: *mut u32 = &mut *available_bytes;
            unsafe {
                let bytes_ptr = ptr::addr_of_mut!(*bytes_u.as_mut_ptr());
                let bytes_ref = bytes_ptr.as_mut().unwrap();

                result =
                    if PeekNamedPipe(stream, None,
                                     0, Some(bytes_ref),
                                     None, None).is_ok() {
                        S_OK
                    } else {
                        Error::from_win32().into()
                    };


                if result.is_err() {
                    let result_msg = result.message();
                    let string = OsString::from(result_msg);
                    return Err(string);
                }
                let num_bytes = bytes_u.assume_init();
                length = min(length, num_bytes);
            }
        } else {
            //let mut size: Box<i64> = Box::new_uninit();
            //let size_ptr: *mut i64 = &mut *size;
            let mut size = MaybeUninit::<i64>::uninit();
            // let size_ptr: *mut i64 = ptr::null_mut();
            unsafe {
                let size_ptr = ptr::addr_of_mut!(*size.as_mut_ptr());
                let size_ref = size_ptr.as_mut().unwrap();
                // let size_ref = *size.as_mut_ptr();
                result = if GetFileSizeEx(stream, size_ref).is_ok() { S_OK } else { Error::from_win32().into() };

                if result.is_err() {
                    let result_msg = result.message();
                    let string = OsString::from(result_msg);
                    return Err(string);
                }
                length = min(length, *size_ptr as u32);
                size.assume_init();
            }
        }
    }

    //let mut buf: Vec<u16> = Vec::with_capacity((length + 1) as usize);
    //buf.fill(1);
    let os_str = "\0".repeat((length + 1) as usize);
    let mut buf_vec: Vec<u8> = os_str.as_str().as_bytes().to_vec();
    let mut chars_read = MaybeUninit::<u32>::uninit();
    //let chars_read: *mut u32 = ptr::null_mut();
    // println!("Length: {}, {}", length, length > 0);
    // if length > 0 {
        unsafe {
            match length {
                0 => {

                }
                _ => {
                    // let chars_read_ptr = chars_read.as_mut_ptr();
                    let chars_read_ptr = ptr::addr_of_mut!(*chars_read.as_mut_ptr());
                    // let chars_read_mut = chars_read_ptr.as_mut();
                    let chars_read_mut = Some(chars_read_ptr);
                    // println!("Blocked here");
                    result =
                        if ReadFile(stream, Some(&mut buf_vec[..]),
                                    chars_read_mut, None).is_ok() {
                            S_OK
                        } else {
                            Error::from_win32().into()
                        };
                    // println!("Unblocked here");

                    if result.is_err() {
                        let result_msg = result.message();
                        let string = OsString::from(result_msg);
                        return Err(string);
                    }
                }
            }
        }
    // }

    // let os_str = OsString::with_capacity(buf_vec.len());
    let mut vec_buf: Vec<u16> = std::iter::repeat(0).take(buf_vec.len()).collect();

    unsafe {
        MultiByteToWideChar(
            CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), &buf_vec[..],
            Some(&mut vec_buf[..]));
    }

    // let non_zeros: Vec<u16> = vec_buf.split(|elem| *elem == 0 as u16).collect();
    let non_zeros_init = Vec::new();
    let non_zeros: Vec<u16> =
        vec_buf
        .split(|x| x == &0)
        .map(|x| x.to_vec())
        .fold(non_zeros_init, |mut acc, mut x| {
            acc.append(&mut x);
            acc
        });
    // let non_zeros: &[u16] = non_zeros_slices.into_iter().reduce(|acc, item| [acc, item].concat()).unwrap();
    let os_str = OsString::from_wide(&non_zeros[..]);
    Ok(os_str)
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
        let succ = PeekNamedPipe(
            stream, None, 0, None, bytes_ref, None).is_ok();

        let total_bytes = bytes.assume_init();
        if succ {
            match is_alive(process) {
                Ok(alive) => {
                    let eof = !alive && total_bytes == 0;
                    Ok(eof)
                },
                Err(_) => Ok(true)
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
    /// Channel used to keep the thread alive.
    reader_alive: mpsc::Sender<bool>,
    /// Channel used to send the process handle to the reading thread.
    reader_process_out: mpsc::Sender<Option<LocalHandle>>,
    /// Handle to the thread used to cache read output.
    cache_thread: Option<thread::JoinHandle<()>>,
    /// Channel used to submit a retrieval request to the cache.
    cache_req: mpsc::Sender<Option<(u32, bool)>>,
    /// Channel used to receive a response from the cache.
    cache_resp: mpsc::Receiver<Result<OsString, OsString>>,
    /// Channel used to keep the cache alive.
    cache_alive: mpsc::Sender<bool>,
}

impl PTYProcess {
    /// Create a new [`PTYProcess`] instance.
    ///
    /// # Arguments
    /// * `conin` - Handle to the process standard input stream
    /// * `conout` - Handle to the process standard output stream
    /// * `using_pipes` - `true` if the streams are Windows named pipes, `false` if they are files.
    ///
    /// # Returns
    /// * `pty` - A new [`PTYProcess`] instance.
    pub fn new(conin: LocalHandle, conout: LocalHandle, using_pipes: bool) -> PTYProcess {
        // Continuous reading thread channels
        let (reader_out_tx, reader_out_rx) = mpsc::channel::<Option<Result<OsString, OsString>>>();
        let (reader_alive_tx, reader_alive_rx) = mpsc::channel::<bool>();
        let (reader_process_tx, reader_process_rx) = mpsc::channel::<Option<LocalHandle>>();

        // Reading cache thread channels
        let (cache_alive_tx, cache_alive_rx) = mpsc::channel::<bool>();
        let (cache_req_tx, cache_req_rx) = mpsc::channel::<Option<(u32, bool)>>();
        let (cache_resp_tx, cache_resp_rx) = mpsc::channel::<Result<OsString, OsString>>();

        let reader_thread = thread::spawn(move || {
            let process_result = reader_process_rx.recv();
            if let Ok(Some(process)) = process_result {
                // let mut alive = reader_alive_rx.recv_timeout(Duration::from_millis(300)).unwrap_or(true);
                // alive = alive && !is_eof(process, conout).unwrap();

                while reader_alive_rx.recv_timeout(Duration::from_millis(100)).unwrap_or(true) {
                    if !is_eof(process.into(), conout.into()).unwrap() {
                        let result = read(4096, true, conout.into(), using_pipes);
                        reader_out_tx.send(Some(result)).unwrap();
                    } else {
                        reader_out_tx.send(None).unwrap();
                    }
                    // alive = reader_alive_rx.recv_timeout(Duration::from_millis(300)).unwrap_or(true);
                    // alive = alive && !is_eof(process, conout).unwrap();
                }
            }

            drop(reader_process_rx);
            drop(reader_alive_rx);
            drop(reader_out_tx);
        });

        let cache_thread = thread::spawn(move || {
            let mut read_buf = OsString::new();
            let mut eof_reached;
            while cache_alive_rx.try_recv().unwrap_or(true) {
                if let Ok(Some((length, blocking))) = cache_req_rx.recv() {
                    let mut pending_read: Option<OsString> = None;

                    if (length as usize) <= read_buf.len() {
                        pending_read = Some(OsString::new());
                    }

                    eof_reached = false;

                    let out =
                        match pending_read {
                            Some(bytes) => Ok(bytes),
                            None => {
                                match blocking {
                                    true => {
                                        match reader_out_rx.recv() {
                                            Ok(None) => {
                                                eof_reached = true;
                                                Ok(OsString::new())
                                            }
                                            Ok(Some(bytes)) => bytes,
                                            Err(_) => Ok(OsString::new())
                                        }
                                    },
                                    false => {
                                        match reader_out_rx.recv_timeout(Duration::from_millis(200)) {
                                            Ok(None) => {
                                                eof_reached = true;
                                                Ok(OsString::new())
                                            }
                                            Ok(Some(bytes)) => bytes,
                                            Err(_) => Ok(OsString::new())
                                        }
                                    }
                                }
                            }
                        };

                    match out {
                        Ok(bytes) => {
                            read_buf.push(bytes);
                            let vec_buf: Vec<u16> = read_buf.encode_wide().collect();
                            let to_read = min(length as usize, vec_buf.len());
                            let (left, right) = vec_buf.split_at(to_read);
                            let to_return = OsString::from_wide(left);
                            read_buf = OsString::from_wide(right);
                            if eof_reached && to_return.is_empty() && length != 0 {
                                cache_resp_tx.send(Err(OsString::from("Standard out reached EOF"))).unwrap();
                            } else {
                                cache_resp_tx.send(Ok(to_return)).unwrap();
                            }
                        },
                        Err(err) => { cache_resp_tx.send(Err(err)).unwrap(); }
                    }
                }
            }

            drop(reader_out_rx);
            drop(cache_alive_rx);
            drop(cache_resp_tx);
        });

        PTYProcess {
            process: LocalHandle(std::ptr::null_mut()),
            conin,
            conout,
            pid: 0,
            close_process: true,
            reading_thread: Some(reader_thread),
            reader_alive: reader_alive_tx,
            reader_process_out: reader_process_tx,
            cache_thread: Some(cache_thread),
            cache_req: cache_req_tx,
            cache_resp: cache_resp_rx,
            cache_alive: cache_alive_tx,
        }
    }

    /// Read at least `length` characters from a process standard output.
    ///
    /// # Arguments
    /// * `length` - Upper limit on the number of characters to read.
    /// * `blocking` - Block the reading thread if no bytes are available.
    ///
    /// # Notes
    /// * If `blocking = false`, then the function will check how much characters are available on
    /// the stream and will read the minimum between the input argument and the total number of
    /// characters available.
    ///
    /// * The bytes returned are represented using a [`OsString`] since Windows operates over
    /// `u16` strings.
    pub fn read(&self, length: u32, blocking: bool) -> Result<OsString, OsString> {
        // read(length, blocking, self.conout, self.using_pipes)
        self.cache_req.send(Some((length, blocking))).unwrap();

        match self.cache_resp.recv() {
            Ok(Ok(bytes)) => Ok(bytes),
            Ok(Err(err)) => Err(err),
            Err(err) => Err(err.to_string().into())
        }
        // let out = self.reader_in.recv().unwrap();
        // out
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
        let result: HRESULT;

        unsafe {
            let required_size = WideCharToMultiByte(
                CP_UTF8, 0, &vec_buf[..], None,
                PCSTR(ptr::null_mut::<u8>()), None);

            let mut bytes_buf: Vec<u8> = std::iter::repeat(0).take((required_size) as usize).collect();

            WideCharToMultiByte(
                CP_UTF8, 0, &vec_buf[..], Some(&mut bytes_buf[..]),
                PCSTR(ptr::null_mut::<u8>()),
                None);

            let mut written_bytes = MaybeUninit::<u32>::uninit();
            let bytes_ptr: *mut u32 = ptr::addr_of_mut!(*written_bytes.as_mut_ptr());
            let bytes_ref = Some(bytes_ptr);
            // let bytes_ref = bytes_ptr.as_mut();

            result =
                if WriteFile(Into::<HANDLE>::into(self.conin), Some(&bytes_buf[..]), bytes_ref, None).is_ok() {
                    S_OK
                } else {
                    Error::from_win32().into()
                };

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }
            let total_bytes = written_bytes.assume_init();
            Ok(total_bytes)
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
                Into::<HANDLE>::into(self.conout), None, 0, bytes_ref, None, None).is_ok();

            let total_bytes = bytes.assume_init();

            if succ {
                let is_alive =
                    match self.is_alive() {
                        Ok(alive) => alive,
                        Err(err) => {
                            return Err(err);
                        }
                    };

                if total_bytes == 0 && !is_alive {
                    succ = false;
                }
            }

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
            Err(err) => Err(err)
        }
    }

    /// Determine if the process is still alive.
    pub fn is_alive(&self) -> Result<bool, OsString> {
        // let mut exit_code: Box<u32> = Box::new_uninit();
        // let exit_ptr: *mut u32 = &mut *exit_code;
        match is_alive(self.process.into()) {
            Ok(alive) => {
                Ok(alive)
            },
            Err(err) => Err(err)
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

}

impl Drop for PTYProcess {
    fn drop(&mut self) {
        unsafe {
            // Unblock thread if it is waiting for a process handle.
            if self.reader_process_out.send(None).is_ok() { }

            // Cancel all pending IO operations on conout
            let _ = CancelIoEx(Into::<HANDLE>::into(self.conout), None);

            // Send instruction to thread to finish
            if self.reader_alive.send(false).is_ok() { }

            // Wait for the thread to be down
            if let Some(thread_handle) = self.reading_thread.take() {
                thread_handle.join().unwrap();
            }

            // Send message to cache to finish
            if self.cache_alive.send(false).is_ok() {}

            // Send a None request to the cache in order to prevent blockage
            if self.cache_req.send(None).is_ok() {}

            // Wait for the cache to be down
            if let Some(cache_handle) = self.cache_thread.take() {
                cache_handle.join().unwrap();
            }

            if !self.conin.is_invalid() {
                let _ = CloseHandle(Into::<HANDLE>::into(self.conin));
            }

            if !self.conout.is_invalid() {
                let _ = CloseHandle(Into::<HANDLE>::into(self.conout));
            }

            if self.close_process && !self.process.is_invalid() {
                let _ = CloseHandle(Into::<HANDLE>::into(self.process));
            }
        }
    }
}
