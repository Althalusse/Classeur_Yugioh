# Fabrique un dossier a donner : l'application, et RIEN d'autre.
#
#   .\Preparer-Une-Distribution.ps1
#
# Produit Distribution\YGO-Collection-Manager\ : les deux executables, un
# lanceur, un LISEZ-MOI. Aucune donnee -- pas de classeur, pas d'image, pas de
# base. Celui qui le recoit demarre sur l'ecran de premier lancement.
#
# Pourquoi si peu de fichiers : les tables de reference (raretes, DDL, regles
# Scanflip) sont compilees DANS le binaire, et les polices a ideogrammes sont
# empruntees au systeme. L'executable se suffit.

$ici    = $PSScriptRoot
$sortie = Join-Path $ici "Distribution\YGO-Collection-Manager"

Push-Location $ici
try {
    Write-Host "Compilation en release..." -ForegroundColor Cyan
    cargo build --release -p ygo-ui -p ygo-cli
    if ($LASTEXITCODE -ne 0) { throw "la compilation a echoue" }
} finally {
    Pop-Location
}

if (Test-Path $sortie) { Remove-Item $sortie -Recurse -Force }
New-Item -ItemType Directory -Path $sortie -Force | Out-Null

foreach ($nom in @("ygo-ui.exe", "ygo-cli.exe")) {
    Copy-Item (Join-Path $ici "target\release\$nom") (Join-Path $sortie $nom) -Force
}

@'
# Demarrer l'application
$exe = Join-Path $PSScriptRoot "ygo-ui.exe"
if ($args.Count -gt 0) { & $exe $PSScriptRoot $args[0] } else { & $exe }
'@ | Set-Content (Join-Path $sortie "Lancer.ps1") -Encoding UTF8

@"
Yu-Gi-Oh! Collection Manager
============================

Pour demarrer : double-cliquez ygo-ui.exe, ou lancez Lancer.ps1.

Au premier demarrage, l'application propose de construire sa base de
reference : environ 60 Mo a telecharger, moins d'une minute. Elle s'installe
dans un sous-dossier bdd\ cree a cote de l'executable.

Ce dossier est deplacable tel quel : l'application cherche toujours ses
donnees a cote de son executable.

Ligne de commande : ygo-cli.exe --help
"@ | Set-Content (Join-Path $sortie "LISEZ-MOI.txt") -Encoding UTF8

Write-Host ""
Write-Host "Distribution prete : $sortie" -ForegroundColor Green
Get-ChildItem $sortie | ForEach-Object {
    Write-Host ("  {0,-18} {1,6} Mo" -f $_.Name, [math]::Round($_.Length / 1MB, 1))
}
Write-Host ""
Write-Host "Aucune donnee dedans : ni classeur, ni image, ni base." -ForegroundColor Yellow
