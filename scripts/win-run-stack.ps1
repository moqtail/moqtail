# win-run-stack.ps1
#
# Run the full MOQtail stack: build Rust, start relay, publisher, and client-js.
# Logs are written to logs\ with one file per component.
#
# Usage:
#   .\scripts\win-run-stack.ps1 [VIDEO_PATH]   Start the full stack
#   .\scripts\win-run-stack.ps1 stop           Stop all running components
#
# Examples:
#   .\scripts\win-run-stack.ps1
#   .\scripts\win-run-stack.ps1 "data\video\Smoking Test.mp4"
#   .\scripts\win-run-stack.ps1 stop

param(
    [string]$VideoPath = "data\video\smoking_test_1080p.mp4"
)

$RootDir  = Split-Path -Parent $PSScriptRoot
$LogDir   = Join-Path $RootDir "logs"
$PidFile  = Join-Path $LogDir ".stack.pids"

# --- Stop command ---
if ($VideoPath -eq "stop") {
    if (-not (Test-Path $PidFile)) {
        Write-Host "No running stack found."
        exit 0
    }
    Write-Host "Stopping MOQtail stack..."
    Get-Content $PidFile | ForEach-Object {
        $parts  = $_ -split "="
        $name   = $parts[0]
        $procId = [int]$parts[1]
        try {
            Get-Process -Id $procId -ErrorAction Stop | Out-Null
            Write-Host "  Stopping $name (PID $procId)..."
            Stop-Process -Id $procId -Force
        } catch {
            Write-Host "  $name (PID $procId) is not running."
        }
    }
    Remove-Item $PidFile -Force
    Write-Host "All stopped."
    exit 0
}

# --- Resolve video path ---
$AbsVideo = $VideoPath
if (-not [System.IO.Path]::IsPathRooted($VideoPath)) {
    $candidate = Join-Path $RootDir $VideoPath
    if (Test-Path $candidate) {
        $AbsVideo = $candidate
    }
}

if (-not (Test-Path $AbsVideo)) {
    Write-Host "Error: Video file not found: $VideoPath"
    Write-Host "Available videos in data\video\:"
    $videoDir = Join-Path $RootDir "data\video"
    if (Test-Path $videoDir) {
        Get-ChildItem $videoDir | ForEach-Object { Write-Host "  $($_.Name)" }
    } else {
        Write-Host "  (none)"
    }
    exit 1
}

# --- Create log directory ---
New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
$Timestamp = Get-Date -Format "yyyyMMdd_HHmmss"

Write-Host "=== MOQtail Stack ==="
Write-Host "  Video:  $AbsVideo"
Write-Host "  Logs:   $LogDir"
Write-Host ""

# --- Build Rust components ---
Write-Host "`[build`] Building Rust components..."
$buildLog = Join-Path $LogDir "build_$Timestamp.log"
$buildArgs = @("build", "--release", "--manifest-path", (Join-Path $RootDir "Cargo.toml"))
& cargo @buildArgs 2>&1 | ForEach-Object { "$_" } | Tee-Object -FilePath $buildLog
if ($LASTEXITCODE -ne 0) {
    Write-Host "`[build`] Build failed. See $buildLog"
    exit 1
}
Write-Host "`[build`] Done."
Write-Host ""

# Reset PID file
Set-Content -Path $PidFile -Value ""

# --- Start relay ---
Write-Host "`[relay`] Starting relay on port 4433..."
$relayLog  = Join-Path $LogDir "relay_$Timestamp.log"
$relayProc = Start-Process -FilePath "cargo" `
    -ArgumentList @(
        "run", "--release",
        "--manifest-path", (Join-Path $RootDir "Cargo.toml"),
        "--bin", "relay", "--",
        "--port", "4433",
        "--cert-file", (Join-Path $RootDir "apps\relay\cert\cert.pem"),
        "--key-file",  (Join-Path $RootDir "apps\relay\cert\key.pem"),
        "--log-folder", $LogDir
    ) `
    -RedirectStandardOutput $relayLog `
    -RedirectStandardError  "$relayLog.err" `
    -PassThru -NoNewWindow
Add-Content -Path $PidFile -Value "relay=$($relayProc.Id)"
Write-Host "`[relay`] PID $($relayProc.Id) - log: $(Split-Path -Leaf $relayLog)"

# Give relay a moment to bind
Start-Sleep -Seconds 2

# --- Start publisher ---
Write-Host "`[publisher`] Starting publisher with: $(Split-Path -Leaf $AbsVideo)"
$pubLog  = Join-Path $LogDir "publisher_$Timestamp.log"
$pubProc = Start-Process -FilePath "cargo" `
    -ArgumentList @(
        "run", "--release",
        "--manifest-path", (Join-Path $RootDir "Cargo.toml"),
        "--bin", "publisher", "--",
        "--video-path", $AbsVideo,
        "--max-variants", "4"
    ) `
    -RedirectStandardOutput $pubLog `
    -RedirectStandardError  "$pubLog.err" `
    -PassThru -NoNewWindow
Add-Content -Path $PidFile -Value "publisher=$($pubProc.Id)"
Write-Host "`[publisher`] PID $($pubProc.Id) - log: $(Split-Path -Leaf $pubLog)"

# Give publisher a moment to connect and start encoding
Start-Sleep -Seconds 3

# --- Start client-js ---
Write-Host "`[client-js`] Starting Vite dev server..."
$clientLog  = Join-Path $LogDir "client-js_$Timestamp.log"
$clientProc = Start-Process -FilePath "npm" `
    -ArgumentList @("run", "--prefix", (Join-Path $RootDir "apps\client-js"), "dev") `
    -RedirectStandardOutput $clientLog `
    -RedirectStandardError  "$clientLog.err" `
    -PassThru -NoNewWindow
Add-Content -Path $PidFile -Value "client-js=$($clientProc.Id)"
Write-Host "`[client-js`] PID $($clientProc.Id) - log: $(Split-Path -Leaf $clientLog)"

Write-Host ""
Write-Host "=== All running ==="
Write-Host "  Relay:     https://127.0.0.1:4433"
Write-Host "  Client-JS: http://localhost:5173"
Write-Host ""
Write-Host "Stop with:  .\scripts\win-run-stack.ps1 stop"
Write-Host "       or:  Ctrl+C"
Write-Host ""

# --- Wait and handle Ctrl+C ---
try {
    while ($true) {
        Start-Sleep -Seconds 2

        $allDead = $true
        Get-Content $PidFile | ForEach-Object {
            $parts = $_ -split "="
            if ($parts.Count -eq 2) {
                $procId = [int]$parts[1]
                if (Get-Process -Id $procId -ErrorAction SilentlyContinue) {
                    $allDead = $false
                }
            }
        }

        if ($allDead) {
            Write-Host "All processes have exited."
            break
        }
    }
} finally {
    # Cleanup on Ctrl+C or natural exit
    if (Test-Path $PidFile) {
        Write-Host ""
        Write-Host "Shutting down..."
        Get-Content $PidFile | ForEach-Object {
            $parts = $_ -split "="
            if ($parts.Count -eq 2) {
                $name   = $parts[0]
                $procId = [int]$parts[1]
                if (Get-Process -Id $procId -ErrorAction SilentlyContinue) {
                    Write-Host "  Stopping $name (PID $procId)..."
                    Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
                }
            }
        }
        Remove-Item $PidFile -Force
        Write-Host "All processes stopped. Logs are in $LogDir"
    }
}
