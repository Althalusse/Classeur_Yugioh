# Fabrique ce qu'il faut pour publier une version :
#
#   .\Preparer-Une-Distribution.ps1                    exe + archive source
#   .\Preparer-Une-Distribution.ps1 -SourceSeulement   l'archive source seule
#
# 1. Distribution\YGO-Collection-Manager\ : les deux executables, un lanceur,
#    un LISEZ-MOI. Aucune donnee -- pas de classeur, pas d'image, pas de base.
#    Celui qui le recoit demarre sur l'ecran de premier lancement. C'est ce qui
#    se joint a la Release GitHub.
#
# 2. Publication\Classeur_Yugioh_v<version>_source.zip : le code a pousser sur
#    le depot (GitHub Desktop). Exactement ce que le .gitignore laisse passer :
#    jamais Application\ (vos classeurs), ni target\, ni Distribution\, ni le
#    journal Construction_Projet_*.md, ni les dossiers _to_delete.
#
# Pourquoi si peu de fichiers dans la distribution : les tables de reference
# (raretes, DDL, regles Scanflip) sont compilees DANS le binaire, et les polices
# a ideogrammes sont empruntees au systeme. L'executable se suffit.

param(
    [switch]$SourceSeulement
)

$ErrorActionPreference = "Stop"
$ici    = $PSScriptRoot
$sortie = Join-Path $ici "Distribution\YGO-Collection-Manager"

# La version du code : le tag de la Release DOIT etre celle-ci. La mise a jour
# integree lit la version marquee dans l'executable et refuse un fichier dont
# la version differe du tag.
$version = (Select-String -Path (Join-Path $ici "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value

# -----------------------------------------------------------------------------
# L'archive source
# -----------------------------------------------------------------------------

function Preparer-Archive-Source {
    # Ce qui part sur le depot : les fichiers de la racine et cinq dossiers.
    $dossiers = @(".cargo", ".github", "assets", "crates", "tests")
    # Un chemin qui traverse l'un de ces dossiers ne part jamais.
    $exclus = @("_to_delete", "_a_supprimer", "target")

    $fichiers = @()
    $fichiers += Get-ChildItem -LiteralPath $ici -File -Force | Where-Object {
        $_.Name -notlike "Construction_Projet_*" -and
        $_.Extension -notin @(".zip", ".rar", ".7z", ".gz") -and
        $_.Name -notlike "*.db-shm" -and $_.Name -notlike "*.db-wal"
    }
    foreach ($d in $dossiers) {
        $chemin = Join-Path $ici $d
        if (-not (Test-Path -LiteralPath $chemin)) { continue }
        $fichiers += Get-ChildItem -LiteralPath $chemin -File -Recurse -Force | Where-Object {
            $relatif = $_.FullName.Substring($ici.Length).TrimStart('\', '/')
            $segments = $relatif -split '[\\/]'
            -not ($segments | Where-Object { $exclus -contains $_ }) -and
            $_.Name -notlike "*.db-shm" -and $_.Name -notlike "*.db-wal"
        }
    }

    $publication = Join-Path $ici "Publication"
    New-Item -ItemType Directory -Path $publication -Force | Out-Null
    $archive = Join-Path $publication "Classeur_Yugioh_v$($version)_source.zip"
    if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::Open($archive, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($f in $fichiers) {
            # Des chemins a barres obliques : c'est ce que tout outil relit.
            $nom = $f.FullName.Substring($ici.Length).TrimStart('\', '/') -replace '\\', '/'
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                $zip, $f.FullName, $nom, [System.IO.Compression.CompressionLevel]::Optimal)
        }
    } finally {
        $zip.Dispose()
    }

    $taille = [math]::Round((Get-Item -LiteralPath $archive).Length / 1MB, 1)
    Write-Host ""
    Write-Host "Archive source prete : $archive" -ForegroundColor Green
    Write-Host ("  {0} fichiers, {1} Mo" -f $fichiers.Count, $taille)
    Write-Host "  A extraire dans le depot local, puis commit et push (GitHub Desktop)." -ForegroundColor Cyan
}

if ($SourceSeulement) {
    Write-Host "Version : $version" -ForegroundColor Green
    Preparer-Archive-Source
    return
}

# -----------------------------------------------------------------------------
# La distribution
# -----------------------------------------------------------------------------

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

# L'empreinte SHA-256 de ygo-ui.exe. GitHub calcule desormais la sienne pour
# chaque fichier joint, et la mise a jour integree s'en sert ; ce fichier reste
# utile aux versions 2.0.0-alpha.2 et anterieures, qui l'exigent, et a qui
# veut verifier a la main (cf. ygo_app::maj_appli).
$exe = Join-Path $sortie "ygo-ui.exe"
$empreinte = (Get-FileHash $exe -Algorithm SHA256).Hash.ToLower()
"$empreinte  ygo-ui.exe" | Set-Content (Join-Path $sortie "ygo-ui.exe.sha256") -Encoding ASCII -NoNewline

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
Write-Host "Aucune donnee dedans : ni classeur, ni image, ni base." -ForegroundColor Yellow

# Le code qui va avec : seulement une fois que la compilation a reussi.
Preparer-Archive-Source

Write-Host ""
Write-Host "Version compilee : $version" -ForegroundColor Green
Write-Host "Pour publier :" -ForegroundColor Cyan
Write-Host "  1. pousser le code de l'archive source (GitHub Desktop)" -ForegroundColor Cyan
Write-Host "  2. Release : tag EXACTEMENT v$version" -ForegroundColor Cyan
Write-Host "     joindre ygo-ui.exe ET ygo-ui.exe.sha256" -ForegroundColor Cyan
Write-Host "     cocher 'pre-release' si c'est une version de test" -ForegroundColor Cyan
Write-Host "Un tag different de la version : la mise a jour sera refusee." -ForegroundColor Yellow
