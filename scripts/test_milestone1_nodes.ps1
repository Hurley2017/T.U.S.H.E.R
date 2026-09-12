$ErrorActionPreference = "Stop"
$env:PATH = "C:\Users\Tusher Mondal\llvm-mingw\bin;C:\Users\Tusher Mondal\.cargo\bin;" + $env:PATH

$binPath = "D:\Projects\T.U.S.H.E.R\target\x86_64-pc-windows-gnullvm\debug\tusher.exe"

# Clean old test data
Remove-Item -Recurse -Force "D:\Projects\T.U.S.H.E.R\.node_a" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force "D:\Projects\T.U.S.H.E.R\.node_b" -ErrorAction SilentlyContinue

Set-Content -Path "in_a.txt" -Value "status`npeers`ninvite`n"
Set-Content -Path "in_b.txt" -Value "status`npeers`n"

Write-Host ">>> Launching Node A (Desktop-A) on port 42801..."
$procA = Start-Process -FilePath $binPath -ArgumentList "--name Desktop-A -p 42801 --discovery-port 42805 -d .node_a --non-interactive" -PassThru -RedirectStandardOutput "out_a.txt" -RedirectStandardError "err_a.txt"

Write-Host ">>> Launching Node B (Tablet-B) on port 42802..."
$procB = Start-Process -FilePath $binPath -ArgumentList "--name Tablet-B -p 42802 --discovery-port 42805 -d .node_b --non-interactive" -PassThru -RedirectStandardOutput "out_b.txt" -RedirectStandardError "err_b.txt"

Start-Sleep -Seconds 6

Write-Host ">>> Node A logs:"
if (Test-Path "out_a.txt") { Get-Content "out_a.txt" }

Write-Host "`n>>> Node B logs:"
if (Test-Path "out_b.txt") { Get-Content "out_b.txt" }

# Cleanup processes
Stop-Process -Id $procA.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $procB.Id -Force -ErrorAction SilentlyContinue
Remove-Item "out_a.txt", "out_b.txt", "err_a.txt", "err_b.txt" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force "D:\Projects\T.U.S.H.E.R\.node_a", "D:\Projects\T.U.S.H.E.R\.node_b" -ErrorAction SilentlyContinue
