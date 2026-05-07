#Requires -Version 5.1
# jinx installer — Windows (x86-64)
# Usage: irm https://raw.githubusercontent.com/ramtoearth/jinx/main/install.ps1 | iex
[CmdletBinding()]
param(
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\jinx"
)

$ErrorActionPreference = "Stop"
$REPO   = "ramtoearth/jinx"
$TARGET = "x86_64-pc-windows-msvc"

# ── Resolve latest release ───────────────────────────────────────────────────
Write-Host "-> Buscando la ultima version de jinx..." -ForegroundColor Cyan
$release = Invoke-RestMethod "https://api.github.com/repos/$REPO/releases/latest"
$VERSION = $release.tag_name

if (-not $VERSION) {
    Write-Error "No se pudo determinar la version mas reciente."
    exit 1
}
Write-Host "   Version: $VERSION"

# ── Download archive + checksum ──────────────────────────────────────────────
$ARCHIVE  = "jinx-$VERSION-$TARGET.zip"
$BASE_URL = "https://github.com/$REPO/releases/download/$VERSION"
$TMP      = [System.IO.Path]::GetTempPath()
$tmpZip   = Join-Path $TMP $ARCHIVE

Write-Host "-> Descargando $ARCHIVE..." -ForegroundColor Cyan
Invoke-WebRequest "$BASE_URL/$ARCHIVE"        -OutFile $tmpZip          -UseBasicParsing
$checksumLine = (Invoke-WebRequest "$BASE_URL/$ARCHIVE.sha256" -UseBasicParsing).Content.Trim()

# ── Verify integrity ─────────────────────────────────────────────────────────
Write-Host "-> Verificando integridad (SHA-256)..." -ForegroundColor Cyan
$expected = ($checksumLine -split '\s+')[0].ToLower()
$actual   = (Get-FileHash $tmpZip -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) {
    Remove-Item $tmpZip -Force
    Write-Error "SHA-256 no coincide. Descarga corrompida o manipulada."
    exit 1
}

# ── Install binary ───────────────────────────────────────────────────────────
Write-Host "-> Instalando en $InstallDir..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Expand-Archive -Path $tmpZip -DestinationPath $InstallDir -Force
Remove-Item $tmpZip -Force

# ── Add to user PATH ─────────────────────────────────────────────────────────
$userPath = [Environment]::GetEnvironmentVariable("PATH", "User") ?? ""
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("PATH", "$InstallDir;$userPath", "User")
    Write-Host "   $InstallDir aniadido a PATH de usuario." -ForegroundColor Green
}

# ── Install uv if missing ────────────────────────────────────────────────────
if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    Write-Host "-> Instalando uv (gestor de entornos Python)..." -ForegroundColor Cyan
    $uvScript = Join-Path $TMP "install-uv.ps1"
    Invoke-WebRequest "https://astral.sh/uv/install.ps1" -OutFile $uvScript -UseBasicParsing
    & $uvScript
    Remove-Item $uvScript -Force
    Write-Host "   uv instalado." -ForegroundColor Green
} else {
    $uvVer = uv --version 2>&1
    Write-Host "-> uv ya esta instalado ($uvVer)." -ForegroundColor Green
}

Write-Host ""
Write-Host "OK  jinx $VERSION instalado en $InstallDir\jinx.exe" -ForegroundColor Green
Write-Host ""
Write-Host "Proximos pasos:"
Write-Host "  1. Abre una nueva terminal para que PATH surta efecto"
Write-Host "  2. Instala Ollama:       https://ollama.com"
Write-Host "  3. Descarga un modelo:   ollama pull llama3.2:3b"
Write-Host "  4. Inicia la app:        jinx"
