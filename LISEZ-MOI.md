# Yu-Gi-Oh! Collection Manager — projet autonome

Ce dossier se suffit à lui-même : les sources, de quoi compiler, de quoi
lancer, de quoi distribuer. Il ne dépend d'aucun autre dossier, et se déplace
d'un glisser.

## Ce qu'il faut avoir

**Rust.** Une seule chose à installer, depuis <https://rustup.rs>. La version
exacte de la chaîne d'outils est épinglée par `rust-toolchain.toml` et
s'installe d'elle-même au premier appel de `cargo`.

Rien d'autre. Ni Python, ni base de données à fournir, ni fichier de
configuration à écrire.

## Démarrer, dans l'ordre

```powershell
.\Compiler.ps1      # quelques minutes la première fois
.\Lancer.ps1        # l'application s'ouvre
```

Au premier démarrage, l'application constate qu'elle n'a pas de base de
référence et propose de la construire : **environ 35 Mo à télécharger, moins
d'une minute**. Elle en profite pour écrire ses priorités de rareté.

Ensuite, `Lancer.ps1` suffit.

## Les six scripts

| Script | Quand | Ce qu'il fait |
|---|---|---|
| `Compiler.ps1` | après chaque modification du code | compile en release, pose les exécutables dans `Application\` |
| `Lancer.ps1` | tous les jours | ouvre l'application |
| `Outils.ps1` | au besoin | la ligne de commande, sur `Application\` |
| `Verifier.ps1` | avant de livrer | tests, clippy, formatage, documentation |
| `Reprendre-Une-Collection.ps1` | une fois | importe une collection existante |
| `Preparer-Une-Distribution.ps1` | pour donner à quelqu'un | un dossier sans aucune donnée |

## Où vivent les données

Dans `Application\`, à côté de l'exécutable :

```
Application\
├── ygo-ui.exe          l'application
├── ygo-cli.exe         la ligne de commande
├── bdd\
│   ├── cardinfo.db     la base de référence (~163 Mo, reconstructible)
│   ├── classeur_creer\ un classeur = un dossier + une base
│   ├── corbeille\      les classeurs supprimés, restaurables
│   ├── app_config.json vos réglages
│   └── rarity_config.json  l'ordre des raretés
├── img\                les illustrations téléchargées
├── export\             vos CSV
└── logs\app.log        ce que l'application a fait
```

L'application cherche toujours ses données **à côté de son exécutable**. C'est
ce qui rend `Application\` déplaçable, copiable et sauvegardable tel quel.

## Reprendre une collection existante

```powershell
.\Reprendre-Une-Collection.ps1 "H:\...\YuGiOh Binder RUST\Installation"
```

Copie classeurs, corbeille, réglages, images et exports. **Ne copie pas**
`cardinfo.db` — elle se reconstruit en dix secondes, et cela épargne 163 Mo.
La source n'est jamais modifiée.

À faire **avant** le premier lancement : la commande refuse d'écrire
par-dessus une installation déjà peuplée.

## Donner l'application à quelqu'un

```powershell
.\Preparer-Une-Distribution.ps1
```

Produit `Distribution\YGO-Collection-Manager\` : les deux exécutables, un
lanceur, un LISEZ-MOI. **Aucune donnée** — ni classeur, ni image, ni base.

Si peu de fichiers parce que tout ce dont l'application a besoin est *dans* le
binaire : les tables de raretés, les schémas SQL, les règles Scanflip y sont
compilées, et les polices à idéogrammes sont empruntées au système.

## Éprouver depuis zéro

Pour voir ce que voit quelqu'un qui installe l'application, sans toucher à vos
données : compilez, puis lancez l'exécutable en lui donnant un dossier vide.

```powershell
.\Application\ygo-ui.exe "C:\Temp\essai"
```

L'écran de premier lancement doit apparaître, la base se construire, et
l'accueil s'ouvrir vide.

## Ce qu'il y a dans le code

| Crate | Rôle |
|---|---|
| `ygo-core` | règles métier pures — aucune entrée/sortie |
| `ygo-db` | accès SQLite, schémas extraits des bases réelles |
| `ygo-sources` | YGOJSON, YGOPRODeck, Yugipedia |
| `ygo-images` | téléchargement, file d'attente, cache |
| `ygo-app` | orchestration — ne connaît aucun type d'interface |
| `ygo-ui` | l'interface, en egui |
| `ygo-cli` | le binaire de diagnostic |

`cargo doc --open` après `.\Verifier.ps1` ouvre la documentation complète : la
plupart des décisions de conception y sont expliquées à l'endroit où elles
s'appliquent.
