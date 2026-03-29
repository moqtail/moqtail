# setup-env.ps1
$VCPKG_ROOT     = "$HOME\vcpkg"
$VCPKG_INSTALL  = "$VCPKG_ROOT\installed\x64-windows"

$env:VCPKG_ROOT        = $VCPKG_ROOT
$env:FFMPEG_DIR        = $VCPKG_INSTALL
$env:FFMPEG_INCLUDE_DIR= "$VCPKG_INSTALL\include"
$env:FFMPEG_LIB_DIR    = "$VCPKG_INSTALL\lib"
$env:LIBCLANG_PATH     = (scoop prefix llvm) + "\bin"
$env:PATH             += ";$VCPKG_INSTALL\bin"

Write-Host "Environment variables set successfully!" -ForegroundColor Green
Write-Host "VCPKG_ROOT:     $env:VCPKG_ROOT"
Write-Host "FFMPEG_DIR:     $env:FFMPEG_DIR"
Write-Host "LIBCLANG_PATH:  $env:LIBCLANG_PATH"
Write-Host "PATH updated with: $VCPKG_INSTALL\bin"