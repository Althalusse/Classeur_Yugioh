# Ligne de commande, sur l'installation de Application\.
#
#   .\Outils.ps1 config              ce que l'application croit savoir
#   .\Outils.ps1 init                construit bdd\cardinfo.db
#   .\Outils.ps1 inventaire          toutes les cartes possedees
#   .\Outils.ps1 schema              verifie les bases, en lecture seule
#   .\Outils.ps1 exporter "C:\...\collection.csv"
#   .\Outils.ps1 --help              la liste complete
#
# Le chemin de l'installation est insere tout seul, apres la commande.

param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Commande,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Reste
)

$app = Join-Path $PSScriptRoot "Application"
$exe = Join-Path $app "ygo-cli.exe"

if (-not (Test-Path $exe)) {
    Write-Host "Application non compilee." -ForegroundColor Red
    Write-Host "Lancez d'abord : .\Compiler.ps1" -ForegroundColor Yellow
    exit 1
}

if ($Commande -in @("--help", "-h", "aide")) { & $exe $Commande }
else { & $exe $Commande $app @Reste }
