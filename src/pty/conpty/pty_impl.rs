/// Actual ConPTY implementation.

use windows::core::{PWSTR, PCWSTR, Error};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE,
    S_OK, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_GENERIC_READ, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, FILE_GENERIC_WRITE,
    FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES};
use windows::Win32::System::Console::{
    HPCON, AllocConsole, GetConsoleWindow,
    GetConsoleMode, CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    SetConsoleMode, SetStdHandle, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE,
    STD_INPUT_HANDLE, COORD, CreatePseudoConsole, ResizePseudoConsole,
    ClosePseudoConsole, FreeConsole};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    PROCESS_INFORMATION, STARTUPINFOEXW, STARTUPINFOW,
    LPPROC_THREAD_ATTRIBUTE_LIST, InitializeProcThreadAttributeList,
    UpdateProcThreadAttribute, CreateProcessW,
    EXTENDED_STARTUPINFO_PRESENT, CREATE_UNICODE_ENVIRONMENT,
    DeleteProcThreadAttributeList};
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
use windows::core::HRESULT;

use std::{mem, ptr};
use std::mem::MaybeUninit;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;

use crate::pty::{PTYProcess, PTYImpl};
use crate::pty::PTYArgs;

/// Struct that contains the required information to spawn a console
/// using the Windows API `CreatePseudoConsole` call.
pub struct ConPTY {
    handle: HPCON,
    process_info: PROCESS_INFORMATION,
    startup_info: STARTUPINFOEXW,
    process: PTYProcess,
    console_allocated: bool
}

unsafe impl Send for ConPTY {}
unsafe impl Sync for ConPTY {}

impl PTYImpl for ConPTY {
    fn new(args: &PTYArgs) -> Result<Box<dyn PTYImpl>, OsString> {
        let mut result: HRESULT;
        if args.cols <= 0 || args.rows <= 0 {
            let err: OsString = OsString::from(format!(
                "PTY cols and rows must be positive and non-zero. Got: ({}, {})", args.cols, args.rows));
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
                conout_pwstr, (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, HANDLE(std::ptr::null_mut()));

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
                FILE_SHARE_READ, None,
                OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0),
                HANDLE(std::ptr::null_mut()));

            if let Err(err) = h_in_res {
                let result_msg = err.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let h_in = h_in_res.unwrap();

            let mut console_mode_un = MaybeUninit::<CONSOLE_MODE>::uninit();
            let console_mode_ref = console_mode_un.as_mut_ptr();

            result =
                if GetConsoleMode(h_console, console_mode_ref.as_mut().unwrap()).is_ok() {
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
            result =
                if SetConsoleMode(h_console, console_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING).is_ok() {
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
            result = if SetStdHandle(STD_OUTPUT_HANDLE, h_console).is_ok() {S_OK} else {Error::from_win32().into()};

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            result = if SetStdHandle(STD_ERROR_HANDLE, h_console).is_ok() {S_OK} else {Error::from_win32().into()};

            if result.is_err() {
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            result = if SetStdHandle(STD_INPUT_HANDLE, h_in).is_ok() {S_OK} else {Error::from_win32().into()};
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
            let size = COORD {X: args.cols as i16, Y: args.rows as i16};

            if !CreatePipe(&mut input_read_side, &mut input_write_side, None, 0).is_ok() {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            if !CreatePipe(&mut output_read_side, &mut output_write_side, None, 0).is_ok() {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            let pty_handle =
                match CreatePseudoConsole(size, input_read_side, output_write_side, 0) {
                    Ok(pty) => pty,
                    Err(err) => {
                        let result_msg = err.message();
                        let string = OsString::from(result_msg);
                        return Err(string);
                    }
                };

            let _ = CloseHandle(input_read_side);
            let _ = CloseHandle(output_write_side);

            let pty_process = PTYProcess::new(input_write_side.into(), output_read_side.into(), true);

            Ok(Box::new(ConPTY {
                handle: pty_handle,
                process_info: PROCESS_INFORMATION::default(),
                startup_info: STARTUPINFOEXW::default(),
                process: pty_process,
                console_allocated
            }) as Box<dyn PTYImpl>)
        }
    }

    fn spawn(&mut self, appname: OsString, cmdline: Option<OsString>, cwd: Option<OsString>, env: Option<OsString>) -> Result<bool, OsString> {
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
                LPPROC_THREAD_ATTRIBUTE_LIST(ptr::null_mut()), 1, 0,
                required_bytes_ptr.as_mut().unwrap());

            // Allocate memory to represent the list
            let mut required_bytes = required_bytes_u.assume_init();
            let mut lp_attribute_list: Box<[u8]> = vec![0; required_bytes].into_boxed_slice();
            let proc_thread_list: LPPROC_THREAD_ATTRIBUTE_LIST = LPPROC_THREAD_ATTRIBUTE_LIST(
                lp_attribute_list.as_mut_ptr().cast::<_>());

            // Prepare Startup Information structure
            let start_info = STARTUPINFOEXW {
                StartupInfo: STARTUPINFOW {
                    cb: mem::size_of::<STARTUPINFOEXW>() as u32,
                    ..Default::default()
                },
                lpAttributeList: proc_thread_list,
            };

            // Initialize the list memory location
            if !InitializeProcThreadAttributeList(start_info.lpAttributeList, 1, 0, &mut required_bytes).is_ok() {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            // Set the pseudoconsole information into the list
            if !UpdateProcThreadAttribute(
                    start_info.lpAttributeList, 0, 0x00020016,
                    Some(self.handle.0 as _), mem::size_of::<HPCON>(),
                    None, None).is_ok() {
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
                PWSTR(cmd),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                Some(environ as _),
                PCWSTR(working_dir),
                si_w_ptr.as_ref().unwrap(),
                &mut self.process_info
            ).is_ok();

            if !succ {
                result = Error::from_win32().into();
                let result_msg = result.message();
                let string = OsString::from(result_msg);
                return Err(string);
            }

            self.process.set_process(self.process_info.hProcess, false);
            Ok(true)
        }
    }

    fn set_size(&self, cols: i32, rows: i32) -> Result<(), OsString> {
        if cols <= 0 || rows <= 0 {
            let err: OsString = OsString::from(format!(
                "PTY cols and rows must be positive and non-zero. Got: ({}, {})", cols, rows));
            return Err(err);
        }

        let size = COORD {X: cols as i16, Y: rows as i16};
        unsafe {
            match ResizePseudoConsole(self.handle, size) {
                Ok(_) => Ok(()),
                Err(err) => {
                    let result_msg = err.message();
                    let string = OsString::from(result_msg);
                    Err(string)
                }
            }
        }
    }

    fn read(&self, length: u32, blocking: bool) -> Result<OsString, OsString> {
        self.process.read(length, blocking)
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
}

impl Drop for ConPTY {
    fn drop(&mut self) {
       unsafe {
            if !self.process_info.hThread.is_invalid() {
                let _ = CloseHandle(self.process_info.hThread);
            }

            if !self.process_info.hProcess.is_invalid() {
                let _ = CloseHandle(self.process_info.hProcess);
            }

            DeleteProcThreadAttributeList(self.startup_info.lpAttributeList);
            ClosePseudoConsole(self.handle);

            if self.console_allocated {
                let _ = FreeConsole();
            }
        }
    }
}
