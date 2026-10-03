# Passe la barriere de qualite du projet.
#
#   .\Verifier.ps1
#
# Les quatre controles qui doivent etre verts avant toute livraison :
# la suite de tests, clippy sans le moindre avertissement, le formatage, et
# la documentation. Compter quelques minutes a froid.

$ici = $PSScriptRoot
$echecs = @()

Push-Location $ici
try {
    Write-Host "1/4  Tests..." -ForegroundColor Cyan
    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { $echecs += "tests" }

    Write-Host "2/4  Clippy..." -ForegroundColor Cyan
    cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { $echecs += "clippy" }

    Write-Host "3/4  Formatage..." -ForegroundColor Cyan
    cargo fmt --all --check
    if ($LASTEXITCODE -ne 0) { $echecs += "formatage" }

    Write-Host "4/4  Documentation..." -ForegroundColor Cyan
    cargo doc --workspace --no-deps
    if ($LASTEXITCODE -ne 0) { $echecs += "documentation" }
} finally {
    Pop-Location
}

Write-Host ""
if ($echecs.Count -eq 0) {
    Write-Host "Les quatre controles sont verts." -ForegroundColor Green
} else {
    Write-Host ("Echecs : " + ($echecs -join ", ")) -ForegroundColor Red
    exit 1
}
