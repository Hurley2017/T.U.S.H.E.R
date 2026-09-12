$ErrorActionPreference = "Stop"
$env:PATH = "C:\Users\Tusher Mondal\llvm-mingw\bin;C:\Users\Tusher Mondal\.cargo\bin;" + $env:PATH

$binPath = "D:\Projects\T.U.S.H.E.R\target\x86_64-pc-windows-gnullvm\debug\tusher.exe"
$dirA = "D:\Projects\T.U.S.H.E.R\.node_a"
$dirB = "D:\Projects\T.U.S.H.E.R\.node_b"
$sharedA = "D:\Projects\T.U.S.H.E.R\test_shared_a"
$sharedB = "D:\Projects\T.U.S.H.E.R\test_shared_b"

# 1. Clean prior runs
Write-Host ">>> Cleaning test environment..."
Remove-Item -Recurse -Force $dirA -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $dirB -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $sharedA -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $sharedB -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force -Path $sharedA | Out-Null
New-Item -ItemType Directory -Force -Path $sharedB | Out-Null

# 2. Populate sample files in Node A's shared folder
Write-Host ">>> Creating sample files in Node A's folder..."
Set-Content -Path (Join-Path $sharedA "project_spec.txt") -Value "T.U.S.H.E.R Decentralized Mesh Specification"
Set-Content -Path (Join-Path $sharedA "notes.md") -Value "# Meeting Notes`nDiscussed distributed SQLite replication."

# 3. Launch Node B (Tablet-B)
Write-Host ">>> Launching Node B (Tablet-B) on TCP 42832, UDP 42835..."
$psiB = New-Object System.Diagnostics.ProcessStartInfo
$psiB.FileName = $binPath
$psiB.Arguments = "--name Tablet-B -p 42832 --discovery-port 42835 -d .node_b"
$psiB.UseShellExecute = $false
$psiB.RedirectStandardInput = $true
$psiB.RedirectStandardOutput = $true
$psiB.RedirectStandardError = $true
$procB = [System.Diagnostics.Process]::Start($psiB)

# 4. Launch Node A (Desktop-A)
Write-Host ">>> Launching Node A (Desktop-A) on TCP 42831, UDP 42835..."
$psiA = New-Object System.Diagnostics.ProcessStartInfo
$psiA.FileName = $binPath
$psiA.Arguments = "--name Desktop-A -p 42831 --discovery-port 42835 -d .node_a"
$psiA.UseShellExecute = $false
$psiA.RedirectStandardInput = $true
$psiA.RedirectStandardOutput = $true
$psiA.RedirectStandardError = $true
$procA = [System.Diagnostics.Process]::Start($psiA)

try {
    # 5. Wait for discovery and connection
    Write-Host ">>> Waiting 5s for discovery & handshake..."
    Start-Sleep -Seconds 5

    # 6. Configure shared folder and index on Node A
    Write-Host ">>> Registering and indexing folder on Node A..."
    $procA.StandardInput.WriteLine("add-folder f_shared MyDocs $sharedA")
    Start-Sleep -Seconds 1
    $procA.StandardInput.WriteLine("index f_shared")
    Start-Sleep -Seconds 1
    $procA.StandardInput.WriteLine("events f_shared")
    Start-Sleep -Seconds 1

    # 7. Configure shared folder on Node B and query manifest from Node A
    Write-Host ">>> Registering folder on Node B and querying manifest from Desktop-A..."
    $procB.StandardInput.WriteLine("add-folder f_shared MyDocs $sharedB")
    Start-Sleep -Seconds 1
    $procB.StandardInput.WriteLine("manifest Desktop-A f_shared")
    Start-Sleep -Seconds 3

    $procA.StandardInput.WriteLine("quit")
    $procB.StandardInput.WriteLine("quit")
    Start-Sleep -Seconds 1
} finally {
    if (-not $procA.HasExited) { $procA.Kill() }
    if (-not $procB.HasExited) { $procB.Kill() }
}

$outA = $procA.StandardOutput.ReadToEnd()
$outB = $procB.StandardOutput.ReadToEnd()

Write-Host "`n=================== NODE A (DESKTOP-A) ==================="
Write-Host $outA

Write-Host "=================== NODE B (TABLET-B) ==================="
Write-Host $outB

# 8. Verification: Node B's SQLite database must exist and contain metadata
Write-Host "`n=================== VERIFICATION ==================="
$dbB = Join-Path $dirB "tusher_metadata.db"
if (Test-Path $dbB) {
    Write-Host "Node B SQLite Database created: $dbB ($((Get-Item $dbB).Length) bytes)"
    if ($outB -match "DOWNLOAD NEEDED.*project_spec.txt" -and $outB -match "DOWNLOAD NEEDED.*notes.md") {
        Write-Host "`n>>> [SUCCESS] Distributed metadata sync & manifest reconciliation succeeded!" -ForegroundColor Green
        exit 0
    } else {
        Write-Host "`n>>> [FAILURE] Expected manifest reconciliation actions not observed in Node B output." -ForegroundColor Red
        exit 1
    }
} else {
    Write-Host "`n>>> [FAILURE] Database $dbB not found." -ForegroundColor Red
    exit 1
}
