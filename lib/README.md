# Windows API metadata

These `.winmd` files provide the default metadata for the Windows API. This is used to
generate the `windows` and `windows-sys` crates. To view the metadata, use a tool
like [ILSpy](https://github.com/icsharpcode/ILSpy).

These files are used to generate the bindings for the call `NtCreateNamedPipeFile`,
which is not being exposed directly in windows-rs at the moment.
