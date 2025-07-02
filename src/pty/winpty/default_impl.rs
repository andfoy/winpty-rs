use std::ffi::OsString;
use crate::pty::{PTYArgs, PTYImpl};

pub struct WinPTY {}

impl PTYImpl for WinPTY {
    fn new(_args: &PTYArgs) -> Result<Box<dyn PTYImpl>, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn spawn(&mut self, _appname: OsString, _cmdline: Option<OsString>, _cwd: Option<OsString>, _env: Option<OsString>) -> Result<bool, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn set_size(&self, _cols: i32, _rows: i32) -> Result<(), OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn read(&self, _blocking: bool) -> Result<OsString, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn write(&mut self, _buf: OsString) -> Result<u32, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn is_eof(&self) -> Result<bool, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn get_exitstatus(&self) -> Result<Option<u32>, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn is_alive(&self) -> Result<bool, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }

    fn get_pid(&self) -> u32 {
        0
    }

    fn get_fd(&self) -> isize {
        -1
    }

    fn wait_for_exit(&self) -> Result<bool, OsString> {
        Err(OsString::from("winpty_rs was compiled without WinPTY enabled"))
    }
}
