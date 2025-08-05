use windows::core::HRESULT;
/// Actual ConPTY implementation.
use windows::core::{Error, Owned, PCWSTR, PWSTR};
use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_CREATE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
    FILE_OPEN_IF, FILE_PIPE_BYTE_STREAM_MODE, FILE_PIPE_BYTE_STREAM_TYPE,
    FILE_PIPE_QUEUE_OPERATION, FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, OBJ_CASE_INSENSITIVE, S_OK, UNICODE_STRING,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ACCESS_RIGHTS, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    SYNCHRONIZE,
};
use windows::Win32::System::Console::{
    AllocConsole, FreeConsole, GetConsoleMode, GetConsoleWindow, SetConsoleMode, SetStdHandle,
    CONSOLE_MODE, COORD, ENABLE_VIRTUAL_TERMINAL_PROCESSING, HPCON, STD_ERROR_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    STARTUPINFOEXW, STARTUPINFOW,
};
use windows::Win32::System::WindowsProgramming::RtlInitUnicodeString;
use windows::Win32::System::IO::IO_STATUS_BLOCK;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
use super::win_bindings::Windows::Wdk::Storage::FileSystem::NtCreateNamedPipeFile;
// use windows_strings::PWSTR;

use std::ffi::{c_void, OsString};
use std::mem::MaybeUninit;
use std::ops::DerefMut;
use std::os::windows::ffi::OsStrExt;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::{mem, ptr, thread};

use super::calls::{ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, ShowHidePseudoConsole};
use crate::pty::PTYArgs;
use crate::pty::{PTYImpl, PTYProcess};

/// Struct that contains the required information to spawn a console
/// using the Windows API `CreatePseudoConsole` call.
pub struct ConPTY {
    handle: Arc<Mutex<(HPCON, bool)>>,
    process_info: PROCESS_INFORMATION,
    startup_info: STARTUPINFOEXW,
    process: PTYProcess,
    console_allocated: bool,
    release_info_tx: mpsc::Sender<(isize, isize, isize, isize, bool)>,
    cleanup_thread: JoinHandle<()>,
    cleanup_tx: mpsc::Sender<bool>
}

fn cleanup(
    // startup_info: LPPROC_THREAD_ATTRIBUTE_LIST,
    handle: isize,
    console_allocated: bool,
) {
    unsafe {
        // DeleteProcThreadAttributeList(startup_info);
        let _ = ClosePseudoConsole(HPCON(handle));

        if console_allocated {
            let _ = FreeConsole();
        }
    }
}

unsafe impl Send for ConPTY {}
unsafe impl Sync for ConPTY {}

impl PTYImpl for ConPTY {
    fn new(args: &PTYArgs) -> Result<Box<dyn PTYImpl>, OsString> {
        let mut result: HRESULT;
        if args.cols <= 0 || args.rows <= 0 {
            let err: OsString = OsString::from(format!(
                "PTY cols and rows must be positive and non-zero. Got: ({}, {})",
                args.cols, args.rows
            ));
            return Err(err);
        }

        unsafe {
            // Create a console window in case ConPTY is running in a GUI application.
            let console_allocated = AllocConsole().is_ok();
            if console_allocated {
                let _ = ShowWindow(GetConsoleWindow(), SW_HIDE).unwrap();
            }

            // Recreate the standard stream inputs in case the parent process
            // has redirected them.
            let conout_name = OsString::from("CONOUT$\0");
            let conout_vec: Vec<u16> = conout_name.encode_wide().collect();
            let conout_pwstr = PCWSTR(conout_vec.as_ptr());

            let h_console_res = CreateFileW(
                conout_pwstr,
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                Some(HANDLE(std::ptr::null_mut())),
            );

            if let Err(err) = h_console_res {
                let result_msg = err.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let h_console = h_console_res.unwrap();

            let conin_name = OsString::from("CONIN$\0");
            let conin_vec: Vec<u16> = conin_name.encode_wide().collect();
            let conin_pwstr = PCWSTR(conin_vec.as_ptr());

            let h_in_res = CreateFileW(
                conin_pwstr,
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                Some(HANDLE(std::ptr::null_mut())),
            );

            if let Err(err) = h_in_res {
                let result_msg = err.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let h_in = h_in_res.unwrap();

            let mut console_mode_un = MaybeUninit::<CONSOLE_MODE>::uninit();
            let console_mode_ref = console_mode_un.as_mut_ptr();

            result = if GetConsoleMode(h_console, console_mode_ref.as_mut().unwrap()).is_ok() {
                S_OK
            } else {
                Error::from_win32().into()
            };

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let console_mode = console_mode_un.assume_init();

            // Enable stream to accept VT100 input sequences
            result = if SetConsoleMode(h_console, console_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING)
                .is_ok()
            {
                S_OK
            } else {
                Error::from_win32().into()
            };

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            // Set new streams
            result = if SetStdHandle(STD_OUTPUT_HANDLE, h_console).is_ok() {
                S_OK
            } else {
                Error::from_win32().into()
            };

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            result = if SetStdHandle(STD_ERROR_HANDLE, h_console).is_ok() {
                S_OK
            } else {
                Error::from_win32().into()
            };

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            result = if SetStdHandle(STD_INPUT_HANDLE, h_in).is_ok() {
                S_OK
            } else {
                Error::from_win32().into()
            };
            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            // Create communication channels
            // - Close these after CreateProcess of child application with pseudoconsole object.
            let mut input_read_side = INVALID_HANDLE_VALUE;
            let mut output_write_side = INVALID_HANDLE_VALUE;

            // - Hold onto these and use them for communication with the child through the pseudoconsole.
            let mut output_read_side = INVALID_HANDLE_VALUE;
            let mut input_write_side = INVALID_HANDLE_VALUE;

            // Setup PTY size
            let size = COORD {
                X: args.cols as i16,
                Y: args.rows as i16,
            };

            let server_desired_access = SYNCHRONIZE
                | FILE_ACCESS_RIGHTS(GENERIC_READ.0)
                | FILE_ACCESS_RIGHTS(GENERIC_WRITE.0);
            let client_desired_access = SYNCHRONIZE
                | FILE_ACCESS_RIGHTS(GENERIC_READ.0)
                | FILE_ACCESS_RIGHTS(GENERIC_WRITE.0);
            let server_share_access = FILE_SHARE_READ | FILE_SHARE_WRITE;
            let client_share_access = FILE_SHARE_READ | FILE_SHARE_WRITE;

            let path_os_str = OsString::from("\\Device\\NamedPipe\\\0");
            let mut path_bytes: Vec<u16> = path_os_str.encode_wide().collect();
            let path_pwstr = PCWSTR(path_bytes.as_mut_ptr());

            // let path_unicode = UNICODE_STRING {
            //     Length: path_bytes.len() as u16,
            //     Buffer: path_pwstr,
            //     ..Default::default()
            // };

            let mut path_unicode_u = MaybeUninit::<UNICODE_STRING>::uninit();
            RtlInitUnicodeString(path_unicode_u.as_mut_ptr(), path_pwstr);
            let path_unicode = path_unicode_u.assume_init();

            let object_attributes = OBJECT_ATTRIBUTES {
                Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
                ObjectName: &path_unicode,
                ..Default::default()
            };

            let mut dir = Owned::new(INVALID_HANDLE_VALUE);

            let mut status_block_u = MaybeUninit::<IO_STATUS_BLOCK>::uninit();
            let status_block_ptr = status_block_u.as_mut_ptr();

            let mut status = NtCreateFile(
                dir.deref_mut(),
                SYNCHRONIZE | FILE_ACCESS_RIGHTS(GENERIC_READ.0),
                &object_attributes,
                status_block_ptr,
                None,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                FILE_OPEN,
                FILE_SYNCHRONOUS_IO_NONALERT,
                None,
                0,
            );

            status_block_u.assume_init_drop();

            if status.is_err() {
                let result = Error::from_hresult(status.into());
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let mut empty_path_u = MaybeUninit::<UNICODE_STRING>::uninit();
            RtlInitUnicodeString(empty_path_u.as_mut_ptr(), PCWSTR::null());
            let empty_path = empty_path_u.assume_init();

            // let empty_path = UNICODE_STRING::default();
            let mut gl_object_attributes = OBJECT_ATTRIBUTES {
                Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
                ObjectName: &empty_path,
                Attributes: OBJ_CASE_INSENSITIVE,
                ..Default::default()
            };

            let status_block_gl_u = MaybeUninit::<IO_STATUS_BLOCK>::uninit();
            let status_block_gl_ptr = status_block_u.as_mut_ptr();
            let mut server_pipe = INVALID_HANDLE_VALUE;
            gl_object_attributes.RootDirectory = *dir;

            // LARGE_INTEGER

            let alt_status = NtCreateNamedPipeFile(
                &mut server_pipe,
                server_desired_access.0.into(),
                &gl_object_attributes,
                status_block_gl_ptr,
                server_share_access.0.into(),
                FILE_CREATE.0.into(),
                0,
                FILE_PIPE_BYTE_STREAM_TYPE.into(),
                FILE_PIPE_BYTE_STREAM_MODE.into(),
                FILE_PIPE_QUEUE_OPERATION.into(),
                1,
                128 * 1024,
                128 * 1024,
                Some(&-10_0000_0000),
            );

            // status_block_gl_u.assume_init();

            if alt_status.is_err() {
                let result = Error::from_hresult(alt_status.into());
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            gl_object_attributes.RootDirectory = server_pipe;
            let mut client_pipe = INVALID_HANDLE_VALUE;
            status = NtCreateFile(
                &mut client_pipe,
                client_desired_access,
                &gl_object_attributes,
                status_block_gl_ptr,
                None,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                client_share_access,
                FILE_OPEN,
                FILE_NON_DIRECTORY_FILE,
                None,
                0,
            );

            status_block_gl_u.assume_init();

            if status.is_err() {
                let result = Error::from_hresult(status.into());
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            if !DuplicateHandle(
                GetCurrentProcess(),
                client_pipe,
                GetCurrentProcess(),
                &mut input_read_side,
                0,
                true,
                DUPLICATE_SAME_ACCESS,
            )
            .is_ok()
            {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            if !DuplicateHandle(
                GetCurrentProcess(),
                client_pipe,
                GetCurrentProcess(),
                &mut output_write_side,
                0,
                true,
                DUPLICATE_SAME_ACCESS,
            )
            .is_ok()
            {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            // if !CreatePipe(&mut input_read_side, &mut input_write_side, None, 0).is_ok() {
            //     result = Error::from_win32().into();
            //     let result_msg = result.message();
            //     let string = OsString::from(result_msg);
            //     return Err(string);
            // }

            // if !CreatePipe(&mut output_read_side, &mut output_write_side, None, 0).is_ok() {
            //     result = Error::from_win32().into();
            //     let result_msg = result.message();
            //     let string = OsString::from(result_msg);
            //     return Err(string);
            // }

            let pty_handle = match CreatePseudoConsole(size, input_read_side, output_write_side, 0)
            {
                Ok(pty) => pty,
                Err(err) => {
                    let result_msg = err.message();
                    let string = OsString::from(result_msg);
                    return Err(string);
                }
            };

            let _ = ShowHidePseudoConsole(pty_handle, false);

            // let _ = CloseHandle(input_read_side);
            // let _ = CloseHandle(output_write_side);

            // let cleanup_callback = Arc::new((Mutex::<Option<Box<dyn Fn() + Send + 'static>>>::new(None), Condvar::new()));
            let (cleanup_tx, cleanup_rx) = mpsc::channel::<bool>();
            let (release_info_tx, release_info_rx) =
                mpsc::channel::<(isize, isize, isize, isize, bool)>();

            let pty_process = PTYProcess::new(
                server_pipe.into(),
                server_pipe.into(),
                true,
                true,
                Some(cleanup_tx.clone()),
            );

            let hpcon_mutex = Arc::new(Mutex::new((pty_handle, true)));
            let hpcon_clone = Arc::clone(&hpcon_mutex);

            let cleanup_thread = thread::spawn(move || {
                let (_hthread_ptr, _hprocess_ptr, _startup_ptr, hpcon_ptr, console_allocated) =
                    release_info_rx.recv().unwrap();
                let clean = cleanup_rx.recv().unwrap();
                if clean {
                    let mut hpcon_guard = hpcon_clone.lock().unwrap();
                    if hpcon_guard.1 {
                        cleanup(
                            // LocalHandle(hthread_ptr as *mut c_void),
                            // LocalHandle(hprocess_ptr as *mut c_void),
                            // LPPROC_THREAD_ATTRIBUTE_LIST(startup_ptr as *mut c_void),
                            hpcon_guard.0.0,
                            console_allocated,
                        );
                        *hpcon_guard = (hpcon_guard.0, false);
                    }
                }
                drop(cleanup_rx);
                drop(release_info_rx);
            });

            Ok(Box::new(ConPTY {
                handle: hpcon_mutex,
                process_info: PROCESS_INFORMATION::default(),
                startup_info: STARTUPINFOEXW::default(),
                process: pty_process,
                console_allocated,
                release_info_tx,
                cleanup_thread,
                cleanup_tx
            }) as Box<dyn PTYImpl>)
        }
    }

    fn spawn(
        &mut self,
        appname: OsString,
        cmdline: Option<OsString>,
        cwd: Option<OsString>,
        env: Option<OsString>,
    ) -> Result<bool, OsString> {
        let result: HRESULT;
        let mut environ: *const u16 = ptr::null();
        let mut working_dir: *const u16 = ptr::null_mut();
        let mut env_buf: Vec<u16>;
        let mut cwd_buf: Vec<u16>;
        let cmd_buf: Vec<u16>;

        let mut cmdline_oss = OsString::new();
        cmdline_oss.clone_from(&appname);
        let mut cmdline_oss_buf: Vec<u16> = cmdline_oss.encode_wide().collect();

        if let Some(env_opt) = env {
            env_buf = env_opt.encode_wide().collect();
            env_buf.push(0);
            environ = env_buf.as_ptr();
        }

        if let Some(cwd_opt) = cwd {
            cwd_buf = cwd_opt.encode_wide().collect();
            cwd_buf.push(0);
            working_dir = cwd_buf.as_ptr();
        }

        if let Some(cmdline_opt) = cmdline {
            cmd_buf = cmdline_opt.encode_wide().collect();
            cmdline_oss_buf.push(0x0020);
            cmdline_oss_buf.extend(cmd_buf);
        }

        cmdline_oss_buf.push(0);
        let cmd = cmdline_oss_buf.as_mut_ptr();

        unsafe {
            // Discover the size required for the list
            let mut required_bytes_u = MaybeUninit::<usize>::uninit();
            let required_bytes_ptr = required_bytes_u.as_mut_ptr();
            let _ = InitializeProcThreadAttributeList(
                Some(LPPROC_THREAD_ATTRIBUTE_LIST(ptr::null_mut())),
                1,
                Some(0),
                required_bytes_ptr.as_mut().unwrap(),
            );

            // Allocate memory to represent the list
            let mut required_bytes = required_bytes_u.assume_init();
            let mut lp_attribute_list: Box<[u8]> = vec![0; required_bytes].into_boxed_slice();
            let proc_thread_list: LPPROC_THREAD_ATTRIBUTE_LIST =
                LPPROC_THREAD_ATTRIBUTE_LIST(lp_attribute_list.as_mut_ptr().cast::<_>());

            // Prepare Startup Information structure
            let start_info = STARTUPINFOEXW {
                StartupInfo: STARTUPINFOW {
                    cb: mem::size_of::<STARTUPINFOEXW>() as u32,
                    ..Default::default()
                },
                lpAttributeList: proc_thread_list,
            };

            // Initialize the list memory location
            if !InitializeProcThreadAttributeList(
                Some(start_info.lpAttributeList),
                1,
                Some(0),
                &mut required_bytes,
            )
            .is_ok()
            {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let handle = self.handle.lock().unwrap();

            // Set the pseudoconsole information into the list
            if !UpdateProcThreadAttribute(
                start_info.lpAttributeList,
                0,
                0x00020016,
                Some(handle.0.0 as _),
                mem::size_of::<HPCON>(),
                None,
                None,
            )
            .is_ok()
            {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            self.startup_info = start_info;
            let si_ptr = &start_info as *const STARTUPINFOEXW;
            let si_ptr_addr = si_ptr as usize;
            let si_w_ptr = si_ptr_addr as *const STARTUPINFOW;

            let succ = CreateProcessW(
                PCWSTR(ptr::null_mut()),
                Some(PWSTR(cmd)),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                Some(environ as _),
                PCWSTR(working_dir),
                si_w_ptr.as_ref().unwrap(),
                &mut self.process_info,
            )
            .is_ok();

            if !succ {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            self.process.set_process(self.process_info.hProcess, false);
            self.release_info_tx
                .send((
                    self.process_info.hProcess.0 as isize,
                    self.process_info.hThread.0 as isize,
                    self.startup_info.lpAttributeList.0 as isize,
                    handle.0.0,
                    self.console_allocated,
                ))
                .unwrap();
            Ok(true)
        }
    }

    fn set_size(&self, cols: i32, rows: i32) -> Result<(), OsString> {
        if cols <= 0 || rows <= 0 {
            let err: OsString = OsString::from(format!(
                "PTY cols and rows must be positive and non-zero. Got: ({}, {})",
                cols, rows
            ));
            return Err(err);
        }

        let size = COORD {
            X: cols as i16,
            Y: rows as i16,
        };
        unsafe {
            let guard =  self.handle.lock().unwrap();
            match ResizePseudoConsole(guard.0, size) {
                Ok(_) => Ok(()),
                Err(err) => {
                    let result_msg = err.message();
                    let string = OsString::from(result_msg);
                    Err(string)
                }
            }
        }
    }

    fn read(&self, blocking: bool) -> Result<OsString, OsString> {
        self.process.read(blocking)
    }

    fn write(&self, buf: OsString) -> Result<u32, OsString> {
        self.process.write(buf)
    }

    fn is_eof(&self) -> Result<bool, OsString> {
        self.process.is_eof()
    }

    fn get_exitstatus(&self) -> Result<Option<u32>, OsString> {
        self.process.get_exitstatus()
    }

    fn is_alive(&self) -> Result<bool, OsString> {
        self.process.is_alive()
    }

    fn get_pid(&self) -> u32 {
        self.process.get_pid()
    }

    fn get_fd(&self) -> isize {
        self.process.get_fd()
    }

    fn wait_for_exit(&self) -> Result<bool, OsString> {
        self.process.wait_for_exit()
    }

    fn cancel_io(&self) -> Result<bool, OsString> {
        self.process.cancel_io()
    }
}

impl Drop for ConPTY {
    fn drop(&mut self) {
        unsafe {
            self.cleanup_tx.send(false).unwrap_or(());

            if !self.process_info.hThread.is_invalid() {
                let _ = CloseHandle(self.process_info.hThread);
            }

            if !self.process_info.hProcess.is_invalid() {
                let _ = CloseHandle(self.process_info.hProcess);
            }

            DeleteProcThreadAttributeList(self.startup_info.lpAttributeList);
            let mut guard =  self.handle.lock().unwrap();
            if guard.1 {
                let _ = ClosePseudoConsole(guard.0);
                *guard = (guard.0, false);
            }

            if self.console_allocated {
                let _ = FreeConsole();
            }
        }
    }
}
