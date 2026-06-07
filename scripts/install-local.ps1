# Sync the local Voxcaster install from this repo: builds the release binary,
# stages it onto PATH, and copies the usage skill into ~/.claude/skills.
# Run from anywhere: pwsh path\to\install-local.ps1   (or just .\scripts\install-local.ps1)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# --- 1. Build the release binary ---
Write-Output "Building voxcaster (release)..."
Push-Location $repo
try { cargo build --release } finally { Pop-Location }

# Resolve the built binary, honouring a redirected CARGO_TARGET_DIR.
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
$src = Join-Path $targetDir 'release\voxcaster.exe'
if (-not (Test-Path $src)) { throw "built binary not found at $src" }

# --- 2. Stage onto PATH (~/.local/bin). The running server may lock the exe;
#         rename it aside, then drop the new one in. New sessions pick it up. ---
$bin = Join-Path $env:USERPROFILE '.local\bin'
New-Item -ItemType Directory -Force -Path $bin | Out-Null
$dst = Join-Path $bin 'voxcaster.exe'
try {
    Copy-Item $src $dst -Force -ErrorAction Stop
} catch {
    $old = "$dst.old"
    Remove-Item $old -Force -ErrorAction SilentlyContinue
    Move-Item $dst $old -Force
    Copy-Item $src $dst -Force
    Write-Output "  (running binary was locked; renamed aside to voxcaster.exe.old)"
}
Write-Output "Installed binary -> $dst"

# --- 3. Copy the usage skill into ~/.claude/skills ---
$skillDst = Join-Path $env:USERPROFILE '.claude\skills\voxcaster'
New-Item -ItemType Directory -Force -Path $skillDst | Out-Null
Copy-Item (Join-Path $repo 'skills\voxcaster\SKILL.md') (Join-Path $skillDst 'SKILL.md') -Force
Write-Output "Installed skill  -> $skillDst\SKILL.md"

Write-Output "`nDone. Restart Claude Code / verity to pick up the new binary."
