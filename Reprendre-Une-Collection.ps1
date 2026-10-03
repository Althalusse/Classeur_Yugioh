# Reprend une collection existante dans Application\.
#
#   .\Reprendre-Une-Collection.ps1 "H:\...\YuGiOh Binder RUST\Installation"
#   .\Reprendre-Une-Collection.ps1 "H:\...\Projet Python\V1.0.4"
#
# Copie les classeurs, la corbeille, les reglages, les images et les exports.
# NE copie pas cardinfo.db : elle se reconstruit en une dizaine de secondes,
# et cela epargne 163 Mo. La source n'est jamais modifiee.
#
# Refuse si Application\ contient deja une installation : melanger deux
# collections donnerait un resultat que personne ne saurait demeler.

param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Source
)

$app = Join-Path $PSScriptRoot "Application"
$exe = Join-Path $app "ygo-cli.exe"

if (-not (Test-Path $exe)) {
    Write-Host "Application non compilee." -ForegroundColor Red
    Write-Host "Lancez d'abord : .\Compiler.ps1" -ForegroundColor Yellow
    exit 1
}
if (-not (Test-Path $Source)) {
    Write-Host "Source introuvable : $Source" -ForegroundColor Red
    exit 1
}

& $exe adopter $Source $app
