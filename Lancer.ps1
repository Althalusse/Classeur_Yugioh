# Lance l'application.
#
#   .\Lancer.ps1              ouvre l'accueil
#   .\Lancer.ps1 RA02         ouvre directement le classeur RA02
#
# Au premier demarrage, l'application propose de construire sa base de
# reference : environ 35 Mo a telecharger, moins d'une minute. Les donnees
# vivent dans Application\ -- bdd\, img\, export\, logs\ -- a cote de
# l'executable. Ce dossier est deplacable tel quel.

param([string]$Classeur = "")

$app = Join-Path $PSScriptRoot "Application"
$exe = Join-Path $app "ygo-ui.exe"

if (-not (Test-Path $exe)) {
    Write-Host "Application non compilee." -ForegroundColor Red
    Write-Host "Lancez d'abord : .\Compiler.ps1" -ForegroundColor Yellow
    exit 1
}

if ($Classeur) { & $exe $app $Classeur } else { & $exe }
