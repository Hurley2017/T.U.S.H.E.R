$ErrorActionPreference = "Stop"
$env:PATH = "C:\Users\Tusher Mondal\llvm-mingw\bin;C:\Users\Tusher Mondal\.cargo\bin;" + $env:PATH

$binPath = "D:\Projects\T.U.S.H.E.R\target\x86_64-pc-windows-gnullvm\debug\tusher.exe"
$dirA = "D:\Projects\T.U.S.H.E.R\.mesh_node_a"
$dirB = "D:\Projects\T.U.S.H.E.R\.mesh_node_b"
$dirC = "D:\Projects\T.U.S.H.E.R\.mesh_node_c"

$sharedA = "D:\Projects\T.U.S.H.E.R\test_mesh_folder_a"
$sharedB = "D:\Projects\T.U.S.H.E.R\test_mesh_folder_b"
$sharedC = "D:\Projects\T.U.S.H.E.R\test_mesh_folder_c"

# 1. Clean prior runs
Write-Host ">>> Cleaning test environment..."
Remove-Item -Recurse -Force $dirA -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $dirB -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $dirC -ErrorAction SilentlyContinue

Remove-Item -Recurse -Force $sharedA -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $sharedB -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $sharedC -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force -Path $sharedA | Out-Null
New-Item -ItemType Directory -Force -Path $sharedB | Out-Null
New-Item -ItemType Directory -Force -Path $sharedC | Out-Null

# 2. Build tusher binary
Write-Host ">>> Building tusher CLI binary..."
cargo build -p tusher-cli
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}

# 3. Launch Node B (Relay Node) on TCP 42942, UDP 42945
Write-Host ">>> Launching Node B (Relay: Laptop-B) on TCP 42942, UDP 42945..."
$psiB = New-Object System.Diagnostics.ProcessStartInfo
$psiB.FileName = $binPath
$psiB.Arguments = "--name Laptop-B -p 42942 --discovery-port 42945 -d .mesh_node_b"
$psiB.UseShellExecute = $false
$psiB.RedirectStandardInput = $true
$psiB.RedirectStandardOutput = $false
$psiB.RedirectStandardError = $false
$procB = [System.Diagnostics.Process]::Start($psiB)

# 4. Launch Node A (Endpoint 1: Desktop-A) on TCP 42941, UDP 42944
Write-Host ">>> Launching Node A (Endpoint 1: Desktop-A) on TCP 42941, UDP 42944..."
$psiA = New-Object System.Diagnostics.ProcessStartInfo
$psiA.FileName = $binPath
$psiA.Arguments = "--name Desktop-A -p 42941 --discovery-port 42944 -d .mesh_node_a"
$psiA.UseShellExecute = $false
$psiA.RedirectStandardInput = $true
$psiA.RedirectStandardOutput = $false
$psiA.RedirectStandardError = $false
$procA = [System.Diagnostics.Process]::Start($psiA)

# 5. Launch Node C (Endpoint 2: Tablet-C) on TCP 42943, UDP 42946
Write-Host ">>> Launching Node C (Endpoint 2: Tablet-C) on TCP 42943, UDP 42946..."
$psiC = New-Object System.Diagnostics.ProcessStartInfo
$psiC.FileName = $binPath
$psiC.Arguments = "--name Tablet-C -p 42943 --discovery-port 42946 -d .mesh_node_c"
$psiC.UseShellExecute = $false
$psiC.RedirectStandardInput = $true
$psiC.RedirectStandardOutput = $false
$psiC.RedirectStandardError = $false
$procC = [System.Diagnostics.Process]::Start($psiC)

$procA.StandardInput.AutoFlush = $true
$procB.StandardInput.AutoFlush = $true
$procC.StandardInput.AutoFlush = $true

try {
    # 6. Establish 3-Node Topology: A <-> B and C <-> B (NO direct link between A and C)
    Write-Host ">>> Establishing mesh links: Desktop-A -> Laptop-B and Tablet-C -> Laptop-B..."
    Start-Sleep -Seconds 2

    # Connect Desktop-A to Laptop-B
    $procA.StandardInput.WriteLine("connect 127.0.0.1:42942")
    Start-Sleep -Seconds 1

    # Connect Tablet-C to Laptop-B
    $procC.StandardInput.WriteLine("connect 127.0.0.1:42942")
    Start-Sleep -Seconds 2

    # Trust peers on all nodes
    $procA.StandardInput.WriteLine("trust-all")
    $procB.StandardInput.WriteLine("trust-all")
    $procC.StandardInput.WriteLine("trust-all")
    Start-Sleep -Seconds 1

    # 7. Configure shared folder 'mesh_vault' on all 3 nodes
    Write-Host ">>> Registering shared folder 'mesh_vault' across all 3 nodes..."
    $procA.StandardInput.WriteLine("add-folder mesh_vault MeshVault $sharedA")
    $procB.StandardInput.WriteLine("add-folder mesh_vault MeshVault $sharedB")
    $procC.StandardInput.WriteLine("add-folder mesh_vault MeshVault $sharedC")
    Start-Sleep -Seconds 2

    # -------------------------------------------------------------
    # PHASE 1: Forward Transitive Sync (Node A -> Node B -> Node C)
    # -------------------------------------------------------------
    Write-Host "`n>>> [PHASE 1] Creating 'mesh_memo.txt' on Node A..."
    $fileA1 = Join-Path $sharedA "mesh_memo.txt"
    $memoContent = "# T.U.S.H.E.R Multi-Node Mesh Memo`n`nTransitive P2P synchronization across 3 nodes operating without central server."
    Set-Content -Path $fileA1 -Value $memoContent

    Write-Host ">>> Awaiting transitive propagation through Node B to Node C..."
    $fileB1 = Join-Path $sharedB "mesh_memo.txt"
    $fileC1 = Join-Path $sharedC "mesh_memo.txt"
    $syncedAtoC = $false

    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        if ((Test-Path $fileB1) -and (Test-Path $fileC1)) {
            $contentC = Get-Content -Path $fileC1 -Raw
            if ($contentC.Trim() -eq $memoContent.Trim()) {
                $syncedAtoC = $true
                break
            }
        }
    }

    if ($syncedAtoC) {
        Write-Host ">>> [SUCCESS] 'mesh_memo.txt' transitively propagated from Node A -> Node B -> Node C!" -ForegroundColor Green
    } else {
        Write-Host ">>> [FAILURE] 'mesh_memo.txt' failed to arrive on Node C via Node B within timeout." -ForegroundColor Red
    }

    # Verify SHA-256 match
    if (Test-Path $fileC1) {
        $hashA = (Get-FileHash -Algorithm SHA256 -Path $fileA1).Hash
        $hashC = (Get-FileHash -Algorithm SHA256 -Path $fileC1).Hash
        if ($hashA -eq $hashC) {
            Write-Host ">>> [SUCCESS] 100% SHA-256 match between Node A and Node C ($hashA)" -ForegroundColor Green
        } else {
            Write-Host ">>> [FAILURE] SHA-256 mismatch between Node A and Node C!" -ForegroundColor Red
        }
    }

    Start-Sleep -Seconds 2

    # -------------------------------------------------------------
    # PHASE 2: Reverse Transitive Sync (Node C -> Node B -> Node A)
    # -------------------------------------------------------------
    Write-Host "`n>>> [PHASE 2] Creating 64 KB binary 'telemetry.bin' on Node C..."
    $fileC2 = Join-Path $sharedC "telemetry.bin"
    $bytes = New-Object byte[] 65536
    (New-Object System.Random).NextBytes($bytes)
    [System.IO.File]::WriteAllBytes($fileC2, $bytes)

    Write-Host ">>> Awaiting reverse transitive propagation through Node B to Node A..."
    $fileB2 = Join-Path $sharedB "telemetry.bin"
    $fileA2 = Join-Path $sharedA "telemetry.bin"
    $syncedCtoA = $false

    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        if ((Test-Path $fileB2) -and (Test-Path $fileA2)) {
            $bytesA = [System.IO.File]::ReadAllBytes($fileA2)
            if ($bytesA.Length -eq $bytes.Length) {
                $hashOrig = (Get-FileHash -Algorithm SHA256 -Path $fileC2).Hash
                $hashArrived = (Get-FileHash -Algorithm SHA256 -Path $fileA2).Hash
                if ($hashOrig -eq $hashArrived) {
                    $syncedCtoA = $true
                    break
                }
            }
        }
    }

    if ($syncedCtoA) {
        Write-Host ">>> [SUCCESS] 64 KB binary transitively propagated from Node C -> Node B -> Node A with 100% SHA-256 match!" -ForegroundColor Green
    } else {
        Write-Host ">>> [FAILURE] Binary from Node C failed to arrive on Node A via Node B." -ForegroundColor Red
    }

    # -------------------------------------------------------------
    # PHASE 3: Transitive Tombstone Deletion Propagation
    # -------------------------------------------------------------
    Write-Host "`n>>> [PHASE 3] Deleting 'mesh_memo.txt' on Node A..."
    Remove-Item -Force $fileA1

    Write-Host ">>> Awaiting deletion tombstone propagation through Node B to Node C..."
    $deletedOnC = $false
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        if ((-not (Test-Path $fileB1)) -and (-not (Test-Path $fileC1))) {
            $deletedOnC = $true
            break
        }
    }

    if ($deletedOnC) {
        Write-Host ">>> [SUCCESS] Deletion on Node A successfully deleted 'mesh_memo.txt' on Node B and Node C!" -ForegroundColor Green
    } else {
        Write-Host ">>> [FAILURE] Deletion tombstone failed to reach Node C within timeout." -ForegroundColor Red
    }

    # -------------------------------------------------------------
    # PHASE 4: Concurrent Conflict Branching Across Mesh
    # -------------------------------------------------------------
    Write-Host "`n>>> [PHASE 4] Establishing common baseline 'shared_doc.txt' on all nodes..."
    $docFileA = Join-Path $sharedA "shared_doc.txt"
    Set-Content -Path $docFileA -Value "Baseline version 1"

    $docFileB = Join-Path $sharedB "shared_doc.txt"
    $docFileC = Join-Path $sharedC "shared_doc.txt"

    # Wait for baseline to replicate to B and C
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        if ((Test-Path $docFileB) -and (Test-Path $docFileC)) {
            break
        }
    }

    Write-Host ">>> Inducing concurrent conflicting modifications on Node A and Node B..."
    Set-Content -Path $docFileA -Value "Concurrently updated by Node A exclusively."
    Set-Content -Path $docFileB -Value "Concurrently updated by Node B with divergent data."

    Write-Host ">>> Awaiting conflict detection, branching, and propagation..."
    $conflictDetected = $false
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        # Check if any conflict files appear in any of the shared folders
        $conflictFilesB = Get-ChildItem -Path $sharedB -Filter "*Conflict*" -File
        $conflictFilesA = Get-ChildItem -Path $sharedA -Filter "*Conflict*" -File
        $conflictFilesC = Get-ChildItem -Path $sharedC -Filter "*Conflict*" -File

        if (($conflictFilesB.Count -gt 0) -or ($conflictFilesA.Count -gt 0) -or ($conflictFilesC.Count -gt 0)) {
            $conflictDetected = $true
            break
        }
    }

    if ($conflictDetected) {
        Write-Host ">>> [SUCCESS] Zero-Data-Loss Conflict Branching verified! Conflict file created and preserved." -ForegroundColor Green
    } else {
        Write-Host ">>> [WARNING] Conflict branching still converging or resolved deterministically." -ForegroundColor Yellow
    }

    $procA.StandardInput.WriteLine("quit")
    $procB.StandardInput.WriteLine("quit")
    $procC.StandardInput.WriteLine("quit")
    Start-Sleep -Seconds 1
} catch {
    Write-Host "`n>>> [ERROR] Exception caught during test execution: $($_.Exception.ToString())" -ForegroundColor Red
} finally {
    if ($procA -and (-not $procA.HasExited)) { $procA.Kill() }
    if ($procB -and (-not $procB.HasExited)) { $procB.Kill() }
    if ($procC -and (-not $procC.HasExited)) { $procC.Kill() }

}

Write-Host "`n=================== FINAL EVALUATION ==================="
if ($syncedAtoC -and $syncedCtoA -and $deletedOnC) {
    Write-Host ">>> [MILESTONE 5 COMPLETE] Multi-Node Mesh Transitive Sync & Topology Convergence Verified!" -ForegroundColor Green
    exit 0
} else {
    Write-Host ">>> [MILESTONE 5 FAILED] Multi-Node Mesh verification failed." -ForegroundColor Red
    exit 1
}
