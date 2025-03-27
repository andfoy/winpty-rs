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
}


fn read(blocking: bool, stream: HANDLE, using_pipes: bool) -> Result<OsString, OsString> {
    let mut result: HRESULT;
    if !blocking {
        if using_pipes {
            let mut bytes_u = MaybeUninit::<u32>::uninit();

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
            }
        } else {
            let mut size = MaybeUninit::<i64>::uninit();
            unsafe {
                let size_ptr = ptr::addr_of_mut!(*size.as_mut_ptr());
                let size_ref = size_ptr.as_mut().unwrap();
                result = if GetFileSizeEx(stream, size_ref).is_ok() { S_OK } else { Error::from_win32().into() };

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

    unsafe {
        let chars_read_ptr = ptr::addr_of_mut!(*chars_read.as_mut_ptr());
        let chars_read_mut = Some(chars_read_ptr);
        result =
            if ReadFile(stream, Some(&mut buf_vec[..]),
                        chars_read_mut, None).is_ok() {
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

    let mut vec_buf: Vec<u16> = std::iter::repeat(0).take(buf_vec.len()).collect();

    unsafe {
        MultiByteToWideChar(
            CP_UTF8, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), &buf_vec[..],
            Some(&mut vec_buf[..]));
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
    /// Channel used to receive a response from the reading thread.
    reader_out_rx: mpsc::Receiver<Option<Result<OsString, OsString>>>,
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
        const BUFFER_SIZE: usize = 32768;  // 32KB buffer
        
        // Keep only the reading thread channel
        let (reader_out_tx, reader_out_rx) = mpsc::channel::<Option<Result<OsString, OsString>>>();
        let (reader_alive_tx, reader_alive_rx) = mpsc::channel::<bool>();
        let (reader_process_tx, reader_process_rx) = mpsc::channel::<Option<LocalHandle>>();

        let reader_thread = thread::spawn(move || {
            let process_result = reader_process_rx.recv();
            if let Ok(Some(process)) = process_result {
                let mut read_buf = Vec::with_capacity(BUFFER_SIZE);
                
                while reader_alive_rx.recv_timeout(Duration::from_micros(100)).unwrap_or(true) {
                    if !is_eof(process.into(), conout.into()).unwrap() {
                        // Pre-allocate buffer
                        read_buf.clear();
                        read_buf.resize(BUFFER_SIZE, 0);
                        
                        let result = read(true, conout.into(), using_pipes);
                        reader_out_tx.send(Some(result)).unwrap();
                    } else {
                        reader_out_tx.send(None).unwrap();
                    }
                }
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
            reader_alive: reader_alive_tx,
            reader_process_out: reader_process_tx,
            reader_out_rx,
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
            true => {
                match self.reader_out_rx.recv() {
                    Ok(None) => Err(OsString::from("Standard out reached EOF")),
                    Ok(Some(bytes)) => bytes,
                    Err(_) => Ok(OsString::new())
                }
            },
            false => {
                match self.reader_out_rx.recv_timeout(Duration::from_micros(200)) {
                    Ok(None) => Err(OsString::from("Standard out reached EOF")),
                    Ok(Some(bytes)) => bytes,
                    Err(_) => Ok(OsString::new())
                }
            }
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
                CP_UTF8, 0, &vec_buf[..], None,
                PCSTR(ptr::null_mut::<u8>()), None);

            let mut bytes_buf: Vec<u8> = std::iter::repeat(0).take((required_size) as usize).collect();

            WideCharToMultiByte(
                CP_UTF8, 0, &vec_buf[..], Some(&mut bytes_buf[..]),
                PCSTR(ptr::null_mut::<u8>()),
                None);

            let mut total_written = 0u32;
            let mut bytes_written = MaybeUninit::<u32>::uninit();
            let bytes_ptr: *mut u32 = ptr::addr_of_mut!(*bytes_written.as_mut_ptr());
            let bytes_ref = Some(bytes_ptr);

            // Write in chunks
            for chunk in bytes_buf.chunks(BUFFER_SIZE) {
                let write_result =
                    if WriteFile(Into::<HANDLE>::into(self.conin), Some(chunk), bytes_ref, None).is_ok() {
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
