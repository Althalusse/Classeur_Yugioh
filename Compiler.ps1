# Compile l'application et la pose dans Application\.
#
#   .\Compiler.ps1
#
# A relancer apres chaque modification du code source. La premiere compilation
# prend plusieurs minutes ; les suivantes quelques secondes.
#
# Prerequis : Rust installe (https://rustup.rs). La version de la chaine
# d'outils est epinglee par rust-toolchain.toml et s'installe toute seule au
# premier appel.

$ici = $PSScriptRoot
$app = Join-Path $ici "Application"

Write-Host "Compilation en release..." -ForegroundColor Cyan
Push-Location $ici
try {
    cargo build --release -p ygo-ui -p ygo-cli
    if ($LASTEXITCODE -ne 0) { throw "la compilation a echoue" }
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Path $app -Force | Out-Null
foreach ($nom in @("ygo-ui.exe", "ygo-cli.exe")) {
    $source = Join-Path $ici "target\release\$nom"
    if (Test-Path $source) {
        Copy-Item $source (Join-Path $app $nom) -Force
        $mo = [math]::Round((Get-Item $source).Length / 1MB, 1)
        Write-Host ("  {0,-14} {1,6} Mo" -f $nom, $mo) -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "Application prete : $app" -ForegroundColor Green
Write-Host "Lancez-la avec : .\Lancer.ps1" -ForegroundColor Cyan
