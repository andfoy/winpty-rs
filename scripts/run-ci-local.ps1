<#
.SYNOPSIS
  Run the library test suite the way CI runs it, including stdio semantics.

.DESCRIPTION
  Plain `cargo test` from a terminal does NOT reproduce CI. On a GitHub
  runner the test process's stdout/stderr are anonymous PIPES that the
  runner reads to EOF. If any grandchild process (a spawned cmd.exe that
  outlives its PTY, say) still holds the pipe's write end, the reader never
  sees EOF and the job hangs even though cargo itself has exited. Writing
  to a console or to a file has no such wait, which is why hangs reproduce
  in CI and not locally.

  This script redirects to real pipes, reads them asynchronously with a
  timeout, and reports three distinct outcomes:

    PASS          - cargo exited 0 and the pipes reached EOF
    TEST-FAILURE  - cargo exited non-zero (a genuine test failure)
    HANG          - cargo exited but the pipes never reached EOF, or cargo
                    never exited at all. This is the CI hang.

  It also reports any orphaned child processes left behind, since those are
  what hold the pipe open.

.PARAMETER TimeoutSec
  Per-phase timeout. Default 420.

.PARAMETER Features
  Cargo features. Defaults to CI's: conpty + winpty.
#>
[CmdletBinding()]
param(
  [int]    $TimeoutSec    = 420,
  [string] $Features      = "conpty winpty",
  # Prefix of an environment that provides winpty (its Library/bin and
  # Library/lib are prepended to PATH), i.e. what CI gets from
  # `conda install -y winpty`. Only needed with the `winpty` feature.
  [string] $WinptyPrefix  = $env:WINPTY_PREFIX
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

if ($WinptyPrefix) {
  foreach ($sub in 'bin', 'lib') {
    $dir = Join-Path (Join-Path $WinptyPrefix 'Library') $sub
    if (Test-Path $dir) { $env:PATH = "$dir;$env:PATH" }
  }
}
if ($Features -match 'winpty' -and -not (Get-Command winpty-agent.exe -ErrorAction SilentlyContinue)) {
  Write-Warning "building with the 'winpty' feature but winpty-agent.exe is not on PATH."
  Write-Warning "pass -WinptyPrefix <conda env prefix> or set WINPTY_PREFIX, else the link step"
  Write-Warning "will fail with unresolved winpty_* symbols."
}

$featureArgs = ($Features -split '\s+' | Where-Object { $_ } | ForEach-Object { "--features $_" }) -join ' '
$cargoArgs   = "test $featureArgs -- --test-threads=1"

Write-Host "repo     : $repo"
Write-Host "command  : cargo $cargoArgs"
Write-Host "stdio    : anonymous pipes, read to EOF (CI-like)"
Write-Host ""

function Get-ChildSnapshot {
  Get-CimInstance Win32_Process -Filter "Name='cmd.exe' OR Name='OpenConsole.exe' OR Name='winpty-agent.exe' OR Name='conhost.exe'" |
    Select-Object ProcessId, Name, ParentProcessId
}
$before = @(Get-ChildSnapshot | ForEach-Object { $_.ProcessId })

$psi = [System.Diagnostics.ProcessStartInfo]::new()
$psi.FileName               = 'cargo'
$psi.Arguments              = $cargoArgs
$psi.UseShellExecute        = $false
$psi.RedirectStandardOutput = $true   # real pipe
$psi.RedirectStandardError  = $true   # real pipe
$psi.WorkingDirectory       = $repo

$proc = [System.Diagnostics.Process]::Start($psi)
# Read both pipes to EOF asynchronously. These tasks only complete when
# EVERY handle to the write end is closed - including any held by an
# orphaned grandchild. That is precisely the CI hang condition.
$outTask = $proc.StandardOutput.ReadToEndAsync()
$errTask = $proc.StandardError.ReadToEndAsync()

$exited    = $proc.WaitForExit($TimeoutSec * 1000)
$pipesEof  = [System.Threading.Tasks.Task]::WaitAll(@($outTask, $errTask), $TimeoutSec * 1000)

$verdict = if (-not $exited) {
  'HANG'
} elseif (-not $pipesEof) {
  'HANG'
} elseif ($proc.ExitCode -ne 0) {
  'TEST-FAILURE'
} else {
  'PASS'
}

if ($outTask.IsCompleted) { $stdout = $outTask.Result } else { $stdout = '<pipe never reached EOF>' }
if ($errTask.IsCompleted) { $stderr = $errTask.Result } else { $stderr = '<pipe never reached EOF>' }

# Surface the parts that matter: per-test lines and failures.
($stdout + "`n" + $stderr) -split "`r?`n" |
  Where-Object { $_ -match '^(running |test |test result|error|warning: unused manifest|failures)' -or $_ -match 'panicked at' } |
  ForEach-Object { "  $_" }

Write-Host ""
Write-Host "process exited : $exited  (exit code: $(if ($exited) { $proc.ExitCode } else { 'n/a' }))"
Write-Host "pipes hit EOF  : $pipesEof"

$after   = @(Get-ChildSnapshot)
$orphans = $after | Where-Object { $_.ProcessId -notin $before } | ForEach-Object {
  $alive = $null -ne (Get-Process -Id $_.ParentProcessId -ErrorAction SilentlyContinue)
  [PSCustomObject]@{ PID = $_.ProcessId; Name = $_.Name; ParentAlive = $alive }
}
if ($orphans) {
  Write-Host ""
  Write-Host "leftover child processes (these are what hold the pipe open):"
  $orphans | ForEach-Object { Write-Host ("  pid={0,-7} {1,-18} parentAlive={2}" -f $_.PID, $_.Name, $_.ParentAlive) }
} else {
  Write-Host "leftover child processes : none"
}

if (-not $exited) { try { $proc.Kill($true) } catch {} }

Write-Host ""
Write-Host "VERDICT: $verdict"
switch ($verdict) {
  'PASS'         { exit 0 }
  'TEST-FAILURE' { exit 1 }
  'HANG'         { exit 2 }
}
