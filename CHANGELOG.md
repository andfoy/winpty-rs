## Version 1.0.7 (2026/10/27)

This release fixes a teardown deadlock that affects **every build made
against ConPTY 1.25.260930003 or newer**, which NuGet has served since
2026-09-30. Because `build.rs` previously resolved the ConPTY package
unpinned, this affects builds of already-published 1.0.6 as well - upgrading
is recommended for all users.

### Issues Closed

* [Issue 133](https://github.com/andfoy/winpty-rs/issues/133) - Async write returns with the last chunk pending over dropped memory, and its count omits completed chunks ([PR 134](https://github.com/andfoy/winpty-rs/pull/134) by [@andfoy](https://github.com/andfoy))
* [Issue 132](https://github.com/andfoy/winpty-rs/issues/132) - cmdline meaning differs between conpty and winpty impls? ([PR 139](https://github.com/andfoy/winpty-rs/pull/139) by [@andfoy](https://github.com/andfoy))

In this release 2 issues were closed.

### Pull Requests Merged

* [PR 140](https://github.com/andfoy/winpty-rs/pull/140) - Reference the ngrok token from secrets instead of inlining it, by [@andfoy](https://github.com/andfoy)
* [PR 139](https://github.com/andfoy/winpty-rs/pull/139) - Make cmdline mean the same thing on both backends, by [@andfoy](https://github.com/andfoy) ([132](https://github.com/andfoy/winpty-rs/issues/132))
* [PR 138](https://github.com/andfoy/winpty-rs/pull/138) - Fix all clippy warnings (90 -> 0), by [@andfoy](https://github.com/andfoy)
* [PR 137](https://github.com/andfoy/winpty-rs/pull/137) - Set STARTF_USESTDHANDLES; drop the SetStdHandle hijack, by [@andfoy](https://github.com/andfoy)
* [PR 136](https://github.com/andfoy/winpty-rs/pull/136) - Fix teardown deadlock with ConPTY 1.25; pin the NuGet version, by [@andfoy](https://github.com/andfoy)
* [PR 134](https://github.com/andfoy/winpty-rs/pull/134) - Fix async write byte-count and OVERLAPPED lifetime bugs, by [@andfoy](https://github.com/andfoy) ([133](https://github.com/andfoy/winpty-rs/issues/133))

In this release 6 pull requests were closed.

### Notes for packagers

* The `Microsoft.Windows.Console.ConPTY` NuGet package is now **pinned** in
  `build.rs` (1.25.260930003) rather than resolved to whatever is newest.
  Override with the `WINPTY_RS_CONPTY_VERSION` environment variable; `latest`
  resolves the newest release. A weekly `ConPTY canary` workflow builds
  against `latest` so an upstream change surfaces as a failing canary rather
  than a mystery hang.
* `PTYProcess::write`'s `cmdline` argument now means the same thing on both
  backends: arguments only, with `appname` supplying `argv[0]`. The WinPTY
  backend previously consumed the caller's first argument as `argv[0]` and
  lost it. Callers already following the ConPTY convention - including
  pywinpty - need no change.
* Children are still not terminated when a PTY is dropped if they outlive
  `ClosePseudoConsole`; this no longer blocks teardown, but it can leave a
  stray child process. Tracked separately.


## Version 1.0.6 (2026/06/10)


### Pull Requests Merged

* [PR 123](https://github.com/andfoy/winpty-rs/pull/123) - CI: cache cargo, grcov, and conda env, by [@andfoy](https://github.com/andfoy)
* [PR 122](https://github.com/andfoy/winpty-rs/pull/122) - Bump windows-bindgen to 0.66 with --no-deps to keep NTSTATUS resolution, by [@andfoy](https://github.com/andfoy)
* [PR 120](https://github.com/andfoy/winpty-rs/pull/120) - Use event-driven teardown for PTYProcess reader thread, by [@andfoy](https://github.com/andfoy)
* [PR 119](https://github.com/andfoy/winpty-rs/pull/119) - Test: Add drop duration tests; Fix drop infinite loop and cleanup_thread panic, by [@XY0797](https://github.com/XY0797)
* [PR 118](https://github.com/andfoy/winpty-rs/pull/118) - Bump codecov/codecov-action from 5 to 7, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 115](https://github.com/andfoy/winpty-rs/pull/115) - Bump conda-incubator/setup-miniconda from 3 to 4, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 114](https://github.com/andfoy/winpty-rs/pull/114) - Bump nuget/setup-nuget from 3 to 4, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 113](https://github.com/andfoy/winpty-rs/pull/113) - Fix logical error in get_error_message function, by [@chmxy1213](https://github.com/chmxy1213)
* [PR 111](https://github.com/andfoy/winpty-rs/pull/111) - Fix building for a windows target on a non-Windows system, by [@chmxy1213](https://github.com/chmxy1213)

In this release 9 pull requests were closed.


## Version 1.0.2 (2025/08/05)


### Pull Requests Merged

* [PR 98](https://github.com/andfoy/winpty-rs/pull/98) - Fix crate publishing, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 1.0.1 (2025/08/05)


### Pull Requests Merged

* [PR 97](https://github.com/andfoy/winpty-rs/pull/97) - Expose NtCreateNamedPipeFile directly in package, remove pin to windows-rs fork, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 1.0.0 (2025/07/25)


### Pull Requests Merged

* [PR 96](https://github.com/andfoy/winpty-rs/pull/96) - Replace ConPTY synchronous pipes by async ones, by [@andfoy](https://github.com/andfoy) ([93](https://github.com/andfoy/winpty-rs/issues/93), [490](https://github.com/andfoy/pywinpty/issues/490), [484](https://github.com/andfoy/pywinpty/issues/484))

In this release 1 pull request was closed.


## Version 0.4.0 (2024/10/17)

### Issues Closed

* [Issue 72](https://github.com/andfoy/winpty-rs/issues/72) - FEATURE REQUEST: Callback or blocking method to wait for exit of spawned process ([PR 79](https://github.com/andfoy/winpty-rs/pull/79) by [@andfoy](https://github.com/andfoy))

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 79](https://github.com/andfoy/winpty-rs/pull/79) - Prevent reading thread from hanging if process has stopped, by [@andfoy](https://github.com/andfoy) ([72](https://github.com/andfoy/winpty-rs/issues/72))

In this release 1 pull request was closed.


## Version 0.3.16 (2024/08/09)


### Pull Requests Merged

* [PR 77](https://github.com/andfoy/winpty-rs/pull/77) - Update windows requirement from 0.57.0 to 0.58.0, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 76](https://github.com/andfoy/winpty-rs/pull/76) - fix: remove 100ms delay on first and subsequent read calls, by [@SilverMira](https://github.com/SilverMira) ([75](https://github.com/andfoy/winpty-rs/issues/75))
* [PR 74](https://github.com/andfoy/winpty-rs/pull/74) - Update windows requirement from 0.56.0 to 0.57.0, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 73](https://github.com/andfoy/winpty-rs/pull/73) - Update windows requirement from 0.54.0 to 0.56.0, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 71](https://github.com/andfoy/winpty-rs/pull/71) - Update windows requirement from 0.53.0 to 0.54.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 5 pull requests were closed.


## Version 0.3.15 (2024/02/23)


### Pull Requests Merged

* [PR 70](https://github.com/andfoy/winpty-rs/pull/70) - Update windows requirement from 0.52.0 to 0.53.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 1 pull request was closed.


## Version 0.3.14 (2024/01/23)


### Pull Requests Merged

* [PR 68](https://github.com/andfoy/winpty-rs/pull/68) - Update which requirement from 4.1.0 to 6.0.0, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 66](https://github.com/andfoy/winpty-rs/pull/66) - Bump conda-incubator/setup-miniconda from 2 to 3, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 65](https://github.com/andfoy/winpty-rs/pull/65) - Update enum-primitive-derive requirement from 0.2.2 to 0.3.0, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 64](https://github.com/andfoy/winpty-rs/pull/64) - Update windows requirement from 0.51.1 to 0.52.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 4 pull requests were closed.


## Version 0.3.13 (2023/10/06)


### Pull Requests Merged

* [PR 62](https://github.com/andfoy/winpty-rs/pull/62) - Bump actions/checkout from 3 to 4, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 61](https://github.com/andfoy/winpty-rs/pull/61) - Update windows requirement from 0.48.0 to 0.51.1, by [@dependabot[bot]](https://github.com/apps/dependabot)
* [PR 60](https://github.com/andfoy/winpty-rs/pull/60) - Fix error message returned by winpty is alway Success, by [@indirection42](https://github.com/indirection42)

In this release 3 pull requests were closed.


## Version 0.3.12 (2023-07-15)


### Pull Requests Merged

* [PR 58](https://github.com/andfoy/winpty-rs/pull/58) - Update windows-rs to v0.48.0, by [@andfoy](https://github.com/andfoy) ([57](https://github.com/andfoy/winpty-rs/issues/57))
* [PR 48](https://github.com/andfoy/winpty-rs/pull/48) - Update windows requirement from 0.43.0 to 0.44.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 2 pull requests were closed.


## Version 0.3.11 (2023/01/02)


### Pull Requests Merged

* [PR 47](https://github.com/andfoy/winpty-rs/pull/47) - Prevent iseof from returning immediately, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 0.3.10 (2023/01/02)


### Pull Requests Merged

* [PR 46](https://github.com/andfoy/winpty-rs/pull/46) - Update windows requirement from 0.40.0 to 0.43.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 1 pull request was closed.


## Version 0.3.9 (2022/09/20)

### Issues Closed

* [Issue 43](https://github.com/andfoy/winpty-rs/issues/43) - Release v0.3.9

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 42](https://github.com/andfoy/winpty-rs/pull/42) - Update windows requirement from 0.39.0 to 0.40.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 1 pull request was closed.


## Version 0.3.8 (2022/07/21)


### Pull Requests Merged

* [PR 41](https://github.com/andfoy/winpty-rs/pull/41) - Make sure that the docs can be compiled from a non-Windows host machine, by [@andfoy](https://github.com/andfoy)
* [PR 40](https://github.com/andfoy/winpty-rs/pull/40) - Update windows requirement from 0.38.0 to 0.39.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 2 pull requests were closed.


## Version 0.3.7 (2022/07/05)


### Pull Requests Merged

* [PR 37](https://github.com/andfoy/winpty-rs/pull/37) - Update windows requirement from 0.29.0 to 0.38.0, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 1 pull request was closed.


## Version 0.3.5 (2022/03/04)

### Issues Closed

* [Issue 27](https://github.com/andfoy/winpty-rs/issues/27) - Release v0.3.5

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 26](https://github.com/andfoy/winpty-rs/pull/26) - PR: Use WaitForSingleObject instead of GetExitCodeProcess to determine if the PTY process ended, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 0.3.4 (2022/03/03)

### Issues Closed

* [Issue 25](https://github.com/andfoy/winpty-rs/issues/25) - Release v0.3.4
* [Issue 23](https://github.com/andfoy/winpty-rs/issues/23) - ConPTY tests are failing on CIs

In this release 2 issues were closed.

### Pull Requests Merged

* [PR 24](https://github.com/andfoy/winpty-rs/pull/24) - PR: Remove CI constraints used with windows-2019, by [@andfoy](https://github.com/andfoy)
* [PR 22](https://github.com/andfoy/winpty-rs/pull/22) - Bump actions/checkout from 2 to 3, by [@dependabot[bot]](https://github.com/apps/dependabot)

In this release 2 pull requests were closed.


## Version 0.3.3 (2022/02/02)

### Issues Closed

* [Issue 18](https://github.com/andfoy/winpty-rs/issues/18) - Release v0.3.3

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 17](https://github.com/andfoy/winpty-rs/pull/17) - PR: Emit newline in process when setting CONPTY_CI, by [@andfoy](https://github.com/andfoy)
* [PR 16](https://github.com/andfoy/winpty-rs/pull/16) - Fix write corruption caused by writing an unintended NUL byte, by [@segevfiner](https://github.com/segevfiner) ([210](https://github.com/spyder-ide/pywinpty/issues/210))

In this release 2 pull requests were closed.


## Version 0.3.2 (2022/01/24)

### Issues Closed

* [Issue 15](https://github.com/andfoy/winpty-rs/issues/15) - Release v0.3.2

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 13](https://github.com/andfoy/winpty-rs/pull/13) - PR: Fix some issues related to wstring creation in winpty spawn arguments, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 0.3.1 (2022/01/21)

### Issues Closed

* [Issue 12](https://github.com/andfoy/winpty-rs/issues/12) - Release v0.3.1

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 11](https://github.com/andfoy/winpty-rs/pull/11) - PR: Fix some unstable issues related to reading testing on CI, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 0.3.0 (2022/01/20)

### Issues Closed

* [Issue 10](https://github.com/andfoy/winpty-rs/issues/10) - Release v0.3.0

In this release 1 issue was closed.

### Pull Requests Merged

* [PR 9](https://github.com/andfoy/winpty-rs/pull/9) - PR: Fix concurrency errors related to read and other safety issues, by [@andfoy](https://github.com/andfoy)
* [PR 8](https://github.com/andfoy/winpty-rs/pull/8) - PR: Improve read performance and rewrite iseof, by [@andfoy](https://github.com/andfoy)

In this release 2 pull requests were closed.


## Version 0.2.0 (2022-01-11)


### Pull Requests Merged

* [PR 6](https://github.com/andfoy/winpty-rs/pull/6) - PR: Add get_pid method to PTY struct, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 0.1.2 (2022-01-11)


### Pull Requests Merged

* [PR 5](https://github.com/andfoy/winpty-rs/pull/5) - PR: Require PTYImpl trait to implement Send and Sync, by [@andfoy](https://github.com/andfoy)

In this release 1 pull request was closed.


## Version 0.1.1 (2022-01-11)

### Issues Closed

* [Issue 4](https://github.com/andfoy/winpty-rs/issues/4) - Fix release workflow to use cargo token

In this release 1 issue was closed.

## Version 0.1.0 (2022-01-11)


### Pull Requests Merged

* [PR 3](https://github.com/andfoy/winpty-rs/pull/3) - PR: Add release instructions and update package metadata, by [@andfoy](https://github.com/andfoy)
* [PR 2](https://github.com/andfoy/winpty-rs/pull/2) - PR: Add CIs and dependabot files, by [@andfoy](https://github.com/andfoy)

In this release 2 pull requests were closed.
