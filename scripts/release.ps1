# Bumps the app version, commits, tags and pushes; the release workflow builds and publishes from the tag.
param([Parameter(Mandatory)][string]$Version)

$ErrorActionPreference = "Stop"
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Use a version like 0.2.0" }
Set-Location (Split-Path $PSScriptRoot)
if (git status --porcelain) { throw "Commit or stash your changes first" }
if ((git branch --show-current) -ne "main") { throw "Releases are cut from main" }
if (git tag --list "v$Version") { throw "Tag v$Version already exists" }

$manifest = Get-Content Cargo.toml -Raw
$updated = [regex]::Replace($manifest, '(?m)^version = "[^"]+"', "version = `"$Version`"", 1)
Set-Content Cargo.toml $updated -NoNewline
cargo check --quiet
if ($LASTEXITCODE -ne 0) { throw "cargo check failed" }

git add Cargo.toml Cargo.lock
git commit -m "Release v$Version"
git tag -a "v$Version" -m "bawkseek v$Version"
git push origin main "v$Version"
Write-Output "Pushed v$Version. The release workflow is building it now."
