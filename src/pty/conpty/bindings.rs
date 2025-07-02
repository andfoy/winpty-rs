use std::ffi::c_void;
/// ConPTY bindings
use std::os::windows::raw::HANDLE;
use windows::Win32::System::Console::COORD;


extern "C" {
    /// Creates a "Pseudo-console" (conpty) with dimensions (in characters)
    ///      provided by the `size` parameter. The caller should provide two handles:
    /// - `hInput` is used for writing input to the pty, encoded as UTF-8 and VT sequences.
    /// - `hOutput` is used for reading the output of the pty, encoded as UTF-8 and VT sequences.
    /// Once the call completes, `phPty` will receive a token value to identify this
    ///      conpty object. This value should be used in conjunction with the other
    ///      Pseudoconsole API's.
    /// `dwFlags` is used to specify optional behavior to the created pseudoconsole.
    /// The flags can be combinations of the following values:
    ///  INHERIT_CURSOR: This will cause the created conpty to attempt to inherit the
    ///      cursor position of the parent terminal application. This can be useful
    ///      for applications like `ssh`, where ssh (currently running in a terminal)
    ///      might want to create a pseudoterminal session for an child application
    ///      and the child inherit the cursor position of ssh.
    ///      The created conpty will immediately emit a "Device Status Request" VT
    ///      sequence to hOutput, that should be replied to on hInput in the format
    ///      "\x1b[<r>;<c>R", where `<r>` is the row and `<c>` is the column of the
    ///      cursor position.
    ///      This requires a cooperating terminal application - if a caller does not
    ///      reply to this message, the conpty will not process any input until it
    ///      does. Most *nix terminals and the Windows Console (after Windows 10
    ///      Anniversary Update) will be able to handle such a message.
    pub fn ConptyCreatePseudoConsole(
        size: COORD,
        hInput: HANDLE,
        hOutput: HANDLE,
        dwFlags: u32,
        hPC: *mut c_void,
    ) -> i32;

    /// Resizes the given conpty to the specified size, in characters.
    pub fn ConptyResizePseudoConsole(hPC: *mut c_void, size: COORD) -> i32;

    /// - Clear the contents of the conpty buffer, leaving the cursor row at the top
    ///   of the viewport.
    /// - This is used exclusively by ConPTY to support GH#1193, GH#1882. This allows
    ///   a terminal to clear the contents of the ConPTY buffer, which is important
    ///   if the user would like to be able to clear the terminal-side buffer.
    pub fn ConptyClearPseudoConsole(hPC: *mut c_void) -> i32;

    /// - Tell the ConPTY about the state of the hosting window. This should be used
    ///   to keep ConPTY's internal HWND state in sync with the state of whatever the
    ///   hosting window is.
    /// - For more information, refer to GH#12515.
    pub fn ConptyShowHidePseudoConsole(hPC: *mut c_void, show: bool) -> i32;

    /// - Sends a message to the pseudoconsole informing it that it should use the
    ///   given window handle as the owner for the conpty's pseudo window. This
    ///   allows the response given to GetConsoleWindow() to be a HWND that's owned
    ///   by the actual hosting terminal's HWND.
    /// - Used to support GH#2988
    pub fn ConptyReparentPseudoConsole(hPC: *mut c_void, newParent: *mut c_void) -> i32;

    /// The \Reference handle ensures that conhost keeps running by keeping the ConDrv server pipe open.
    /// After you've finished setting up your PTY via PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, this method may be called
    /// to release that handle, allowing conhost to shut down automatically once the last client has disconnected.
    /// You'll know when this happens, because a ReadFile() on the output pipe will return ERROR_BROKEN_PIPE.
    pub fn ConptyReleasePseudoConsole(hPC: *mut c_void) -> i32;

    /// Closes the conpty and all associated state.
    /// Client applications attached to the conpty will also behave as though the
    ///      console window they were running in was closed.
    /// This can fail if the conhost hosting the pseudoconsole failed to be
    ///      terminated, or if the pseudoconsole was already terminated.
    /// Waits for conhost/OpenConsole to exit first.
    pub fn ConptyClosePseudoConsole(hPC: *mut c_void) -> i32;

    // Packs loose handle information for an inbound ConPTY
    //  session into the same HPCON as a created session.
    pub fn ConptyPackPseudoConsole(
        hServerProcess: HANDLE,
        hRef: HANDLE,
        hSignal: HANDLE,
        phPC: *mut c_void,
    ) -> i32;
}
