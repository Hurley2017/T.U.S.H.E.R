$ErrorActionPreference = "Stop"
$env:PATH = "C:\Users\Tusher Mondal\llvm-mingw\bin;C:\Users\Tusher Mondal\.cargo\bin;" + $env:PATH

$binPath = "D:\Projects\T.U.S.H.E.R\target\x86_64-pc-windows-gnullvm\debug\tusher.exe"
$dirA = "D:\Projects\T.U.S.H.E.R\.sync_node_a"
$dirB = "D:\Projects\T.U.S.H.E.R\.sync_node_b"
$sharedA = "D:\Projects\T.U.S.H.E.R\test_sync_folder_a"
$sharedB = "D:\Projects\T.U.S.H.E.R\test_sync_folder_b"

# 1. Clean prior runs
Write-Host ">>> Cleaning test environment..."
Remove-Item -Recurse -Force $dirA -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $dirB -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $sharedA -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $sharedB -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force -Path $sharedA | Out-Null
New-Item -ItemType Directory -Force -Path $sharedB | Out-Null

# 2. Build tusher binary
Write-Host ">>> Building tusher CLI binary..."
cargo build -p tusher-cli
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}

# 3. Launch Node B (Laptop-B)
Write-Host ">>> Launching Node B (Laptop-B) on TCP 42932, UDP 42935..."
$psiB = New-Object System.Diagnostics.ProcessStartInfo
$psiB.FileName = $binPath
$psiB.Arguments = "--name Laptop-B -p 42932 --discovery-port 42935 -d .sync_node_b"
$psiB.UseShellExecute = $false
$psiB.RedirectStandardInput = $true
$psiB.RedirectStandardOutput = $true
$psiB.RedirectStandardError = $true
$procB = [System.Diagnostics.Process]::Start($psiB)

# 4. Launch Node A (Desktop-A)
Write-Host ">>> Launching Node A (Desktop-A) on TCP 42931, UDP 42935..."
$psiA = New-Object System.Diagnostics.ProcessStartInfo
$psiA.FileName = $binPath
$psiA.Arguments = "--name Desktop-A -p 42931 --discovery-port 42934 -d .sync_node_a"
$psiA.UseShellExecute = $false
$psiA.RedirectStandardInput = $true
$psiA.RedirectStandardOutput = $true
$psiA.RedirectStandardError = $true
$procA = [System.Diagnostics.Process]::Start($psiA)

$procA.StandardInput.AutoFlush = $true
$procB.StandardInput.AutoFlush = $true

try {
    # 5. Wait and connect
    Write-Host ">>> Establishing direct loopback connection from Desktop-A to Laptop-B (127.0.0.1:42932)..."
    Start-Sleep -Seconds 2
    $procA.StandardInput.WriteLine("connect 127.0.0.1:42932")
    Start-Sleep -Seconds 3

    $procA.StandardInput.WriteLine("trust-all")
    $procB.StandardInput.WriteLine("trust-all")
    Start-Sleep -Seconds 1

    # 6. Configure shared folder on both nodes
    Write-Host ">>> Registering shared folder 'mesh_vault' on Node A and Node B..."
    $procA.StandardInput.WriteLine("add-folder mesh_vault MeshVault $sharedA")
    Start-Sleep -Seconds 1
    $procB.StandardInput.WriteLine("add-folder mesh_vault MeshVault $sharedB")
    Start-Sleep -Seconds 2

    # 7. PHASE 1: Write file on Node A -> Verify automated live sync to Node B
    Write-Host "`n>>> [TEST 1] Creating 'architecture_doc.md' in Node A's folder..."
    $fileA1 = Join-Path $sharedA "architecture_doc.md"
    $docContent = "# T.U.S.H.E.R Live Architecture`n`nDecentralized peer-to-peer automated bidirectional sync engine operating smoothly."
    Set-Content -Path $fileA1 -Value $docContent

    Write-Host ">>> Waiting for native watcher & automated transfer to Node B..."
    $fileB1 = Join-Path $sharedB "architecture_doc.md"
    $syncedAtoB = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        if (Test-Path $fileB1) {
            $contentB = Get-Content -Path $fileB1 -Raw
            if ($contentB.Trim() -eq $docContent.Trim()) {
                $syncedAtoB = $true
                break
            }
        }
    }

    if ($syncedAtoB) {
        Write-Host ">>> [SUCCESS] 'architecture_doc.md' automatically synchronized to Node B!" -ForegroundColor Green
    } else {
        Write-Host ">>> [FAILURE] 'architecture_doc.md' failed to arrive on Node B within timeout." -ForegroundColor Red
    }

    # 8. PHASE 2: Write binary file on Node B -> Verify automated live sync to Node A
    Write-Host "`n>>> [TEST 2] Creating 64 KB binary 'data_blob.bin' in Node B's folder..."
    $fileB2 = Join-Path $sharedB "data_blob.bin"
    $bytes = New-Object byte[] 65536
    (New-Object System.Random).NextBytes($bytes)
    [System.IO.File]::WriteAllBytes($fileB2, $bytes)

    Write-Host ">>> Waiting for native watcher & automated transfer to Node A..."
    $fileA2 = Join-Path $sharedA "data_blob.bin"
    $syncedBtoA = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        if (Test-Path $fileA2) {
            $bytesA = [System.IO.File]::ReadAllBytes($fileA2)
            if ($bytesA.Length -eq $bytes.Length) {
                $hashOrig = (Get-FileHash -Algorithm SHA256 -Path $fileB2).Hash
                $hashArrived = (Get-FileHash -Algorithm SHA256 -Path $fileA2).Hash
                if ($hashOrig -eq $hashArrived) {
                    $syncedBtoA = $true
                    break
                }
            }
        }
    }

    if ($syncedBtoA) {
        Write-Host ">>> [SUCCESS] 64 KB binary 'data_blob.bin' automatically synchronized to Node A with 100% SHA-256 match!" -ForegroundColor Green
    } else {
        Write-Host ">>> [FAILURE] 'data_blob.bin' failed to arrive on Node A within timeout." -ForegroundColor Red
    }

    # 9. PHASE 3: Delete file on Node A -> Verify automated deletion sync on Node B
    Write-Host "`n>>> [TEST 3] Deleting 'architecture_doc.md' on Node A..."
    Remove-Item -Force $fileA1

    Write-Host ">>> Waiting for deletion tombstone propagation to Node B..."
    $deletedOnB = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        if (-not (Test-Path $fileB1)) {
            $deletedOnB = $true
            break
        }
    }

    if ($deletedOnB) {
        Write-Host ">>> [SUCCESS] 'architecture_doc.md' automatically deleted on Node B!" -ForegroundColor Green
    } else {
        Write-Host ">>> [FAILURE] 'architecture_doc.md' was not deleted on Node B within timeout." -ForegroundColor Red
    }

    $procA.StandardInput.WriteLine("quit")
    $procB.StandardInput.WriteLine("quit")
    Start-Sleep -Seconds 1
} finally {
    if (-not $procA.HasExited) { $procA.Kill() }
    if (-not $procB.HasExited) { $procB.Kill() }
}

$outA = $procA.StandardOutput.ReadToEnd()
$outB = $procB.StandardOutput.ReadToEnd()

Write-Host "`n=================== NODE A (DESKTOP-A) LOGS ==================="
Write-Host $outA

Write-Host "=================== NODE B (LAPTOP-B) LOGS ==================="
Write-Host $outB

Write-Host "`n=================== FINAL EVALUATION ==================="
if ($syncedAtoB -and $syncedBtoA -and $deletedOnB) {
    Write-Host ">>> [MILESTONE 4 COMPLETE] Full Automated Two-Way Sync Verified Across Processes!" -ForegroundColor Green
    exit 0
} else {
    Write-Host ">>> [MILESTONE 4 FAILED] One or more sync verifications failed." -ForegroundColor Red
    exit 1
}
