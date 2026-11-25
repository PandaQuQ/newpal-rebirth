# Run a command at Idle priority on 2 cores (the user may be gaming).
# Usage: .\run-throttled.ps1 -Log <logBase> -Exe <exe> -ArgLine "<args>"
param([string]$Log, [string]$Exe, [string]$ArgLine)
$env:CARGO_BUILD_JOBS = '2'
$p = Start-Process $Exe -ArgumentList $ArgLine -PassThru -NoNewWindow -RedirectStandardOutput "$Log.out" -RedirectStandardError "$Log.err"
try { $p.PriorityClass = 'Idle'; $p.ProcessorAffinity = [IntPtr]3 } catch {}
$p.WaitForExit()
"exit $($p.ExitCode)" | Out-File "$Log.exit"