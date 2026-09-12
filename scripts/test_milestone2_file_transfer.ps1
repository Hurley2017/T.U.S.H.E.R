$ErrorActionPreference = "Stop"
$env:PATH = "C:\Users\Tusher Mondal\llvm-mingw\bin;C:\Users\Tusher Mondal\.cargo\bin;" + $env:PATH

$binPath = "D:\Projects\T.U.S.H.E.R\target\x86_64-pc-windows-gnullvm\debug\tusher.exe"
$testDataDir = "D:\Projects\T.U.S.H.E.R\test_data"
$sourceFile = Join-Path $testDataDir "sample_document.bin"
$destFile = "D:\Projects\T.U.S.H.E.R\.node_b\downloads\sample_document.bin"

# 1. Clean prior runs
Write-Host ">>> Cleaning previous test data..."
Remove-Item -Recurse -Force "D:\Projects\T.U.S.H.E.R\.node_a" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force "D:\Projects\T.U.S.H.E.R\.node_b" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $testDataDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $testDataDir | Out-Null

# 2. Generate 4.5 MB test payload
Write-Host ">>> Generating 4.5 MB test payload ($sourceFile)..."
$bytes = New-Object byte[] (4500000)
$rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
$rng.GetBytes($bytes)
[System.IO.File]::WriteAllBytes($sourceFile, $bytes)

$sourceHash = (Get-FileHash -Path $sourceFile -Algorithm SHA256).Hash
Write-Host ">>> Source File SHA-256: $sourceHash"

# 3. Launch Node B (Receiver: Tablet-B)
Write-Host ">>> Launching Node B (Tablet-B) on TCP 42822, UDP 42825..."
$psiB = New-Object System.Diagnostics.ProcessStartInfo
$psiB.FileName = $binPath
$psiB.Arguments = "--name Tablet-B -p 42822 --discovery-port 42825 -d .node_b --non-interactive"
$psiB.UseShellExecute = $false
$psiB.RedirectStandardOutput = $true
$psiB.RedirectStandardError = $true
$procB = [System.Diagnostics.Process]::Start($psiB)

# 4. Launch Node A (Sender: Desktop-A)
Write-Host ">>> Launching Node A (Desktop-A) on TCP 42821, UDP 42825..."
$psiA = New-Object System.Diagnostics.ProcessStartInfo
$psiA.FileName = $binPath
$psiA.Arguments = "--name Desktop-A -p 42821 --discovery-port 42825 -d .node_a"
$psiA.UseShellExecute = $false
$psiA.RedirectStandardInput = $true
$psiA.RedirectStandardOutput = $true
$psiA.RedirectStandardError = $true
$procA = [System.Diagnostics.Process]::Start($psiA)

try {
    # 5. Wait for discovery and connection establishment
    Write-Host ">>> Waiting 5s for automatic LAN UDP discovery & handshake..."
    Start-Sleep -Seconds 5

    Write-Host ">>> Querying peers on Node A..."
    $procA.StandardInput.WriteLine("peers")
    Start-Sleep -Seconds 2

    # 6. Send file from Node A to Node B
    Write-Host ">>> Sending payload from Node A to Tablet-B..."
    $procA.StandardInput.WriteLine("send Tablet-B $sourceFile")
    
    # 7. Wait for transfer to complete
    Start-Sleep -Seconds 6

    $procA.StandardInput.WriteLine("quit")
    Start-Sleep -Seconds 1
} finally {
    if (-not $procA.HasExited) { $procA.Kill() }
    if (-not $procB.HasExited) { $procB.Kill() }
}

$outA = $procA.StandardOutput.ReadToEnd()
$outB = $procB.StandardOutput.ReadToEnd()
$errA = $procA.StandardError.ReadToEnd()
$errB = $procB.StandardError.ReadToEnd()

Write-Host "`n=================== NODE A OUTPUT ==================="
Write-Host $outA
if ($errA) { Write-Host "ERRORS A: $errA" }

Write-Host "=================== NODE B OUTPUT ==================="
Write-Host $outB
if ($errB) { Write-Host "ERRORS B: $errB" }

# 8. Verify integrity on Node B
Write-Host "`n=================== VERIFICATION ==================="
if (Test-Path $destFile) {
    $destHash = (Get-FileHash -Path $destFile -Algorithm SHA256).Hash
    Write-Host "Destination File: $destFile"
    Write-Host "Destination Size: $((Get-Item $destFile).Length) bytes"
    Write-Host "Destination SHA-256: $destHash"

    if ($destHash -eq $sourceHash) {
        Write-Host "`n>>> [SUCCESS] File transfer succeeded! Cryptographic hashes match 100%!" -ForegroundColor Green
        exit 0
    } else {
        Write-Host "`n>>> [FAILURE] Hash mismatch!" -ForegroundColor Red
        Write-Host "Expected: $sourceHash"
        Write-Host "Actual:   $destHash"
        exit 1
    }
} else {
    Write-Host "`n>>> [FAILURE] Destination file not found at $destFile!" -ForegroundColor Red
    exit 1
}
