# Classeur Yu-Gi-Oh! — Yu-Gi-Oh! Collection Manager

Gérer sa collection de cartes Yu-Gi-Oh! **set par set**, comme dans un vrai
classeur : chaque tirage à sa place, avec sa rareté, son illustration et son
édition, et pour chaque carte possédée, l'état de chaque exemplaire.

Application de bureau pour **Windows**, écrite en **Rust** (interface egui).
Version actuelle : **2.0.0-alpha.1**.

> Projet de fan, non officiel, sans lien avec Konami. *Yu-Gi-Oh!* et les
> illustrations des cartes appartiennent à leurs ayants droit. Aucune image de
> carte n'est distribuée avec l'application : elles sont téléchargées depuis les
> sources publiques au moment où on les affiche.

---

## Ce que fait l'application

- **Un classeur par set** — toutes les cartes du set, page par page, dans
  l'ordre que vous choisissez (numéro, rareté, type…). Recherche, filtres,
  saut de page, double page.
- **L'image de chaque tirage** — la vraie carte de *votre* tirage, rareté,
  langue et édition comprises (source Yugipedia), avec repli sur YGOPRODeck
  quand l'image manque.
- **Les illustrations alternatives** — repérées, proposées, et vérifiées contre
  la liste officielle du set avant de vous être montrées.
- **Les exemplaires** — cinq exemplaires d'une même carte peuvent avoir chacun
  leur état (Mint, Near Mint…) et leur édition (1st, Unlimited, Limited). Par
  défaut, une carte qui entre dans la collection est Mint, avec l'édition que
  la base connaît pour ce tirage.
- **L'inventaire** — toute la collection, tous classeurs confondus : tri,
  filtres, sélection multiple, actions en masse, détail par exemplaire.
- **Import / export Scanflip** — aller-retour CSV sans perte d'état ni
  d'édition.
- **Noms français officiels** — jamais inventés : une carte sans nom français
  officiel garde son nom anglais.
- **Statistiques**, **corbeille** (un classeur supprimé se restaure), **fiche
  de carte**.
- **Mise à jour de la base** — un bandeau prévient quand une nouvelle version
  des données est disponible ; elle se reconstruit en quelques secondes.

## Installer

1. Ouvrez l'onglet **[Releases](../../releases)** et téléchargez la dernière
   version (`.zip`).
2. Décompressez-la où vous voulez.
3. Lancez **`ygo-ui.exe`**.

Au premier démarrage, l'application construit sa base de référence : **environ
60 Mo à télécharger, moins d'une minute**. Tout s'installe à côté de
l'exécutable — le dossier se déplace et se sauvegarde tel quel.

Ligne de commande (diagnostic, réparations, import/export) : `ygo-cli.exe --help`.

## D'où viennent les données — et comment on les respecte

| Source | Ce qu'elle fournit | Règles appliquées |
|---|---|---|
| [YGOJSON](https://github.com/iconmaster5326/YGOJSON) | cartes, sets, tirages, textes multilingues | une archive par mise à jour |
| [YGOPRODeck](https://ygoprodeck.com/api-guide/) | catalogue, statistiques, images par carte | ≤ 15 requêtes/s (limite : 20), données gardées en local, chaque image téléchargée **une seule fois** |
| [Yugipedia](https://yugipedia.com/wiki/Yugipedia:API) | image de chaque tirage, listes officielles des sets | ≤ 1 requête/s, **une seule requête à la fois**, cache de 30 jours, requêtes regroupées par 50, User-Agent avec contact |

Ces règles sont tenues **pour l'ensemble du programme** (quotas et cache
partagés par tous ses fils), parce que l'application est partagée : elles
doivent tenir pour cent utilisateurs comme pour un. Une adresse introuvable
n'est pas redemandée avant sept jours.

Un problème avec ces accès ? Ouvrez un ticket sur ce dépôt — c'est le contact
déclaré dans le User-Agent.

## Compiler depuis les sources

Il faut **Rust** (<https://rustup.rs>) ; la version exacte est épinglée par
`rust-toolchain.toml` et s'installe d'elle-même. Rien d'autre.

```powershell
.\Compiler.ps1                      # compile, pose les exécutables dans Application\
.\Lancer.ps1                        # ouvre l'application
.\Verifier.ps1                      # tests, clippy, formatage, documentation
.\Preparer-Une-Distribution.ps1     # un dossier à donner, sans aucune donnée
```

Le détail des six scripts et de l'emplacement des données est dans
[`LISEZ-MOI.md`](LISEZ-MOI.md).

### Qualité

Chaque envoi sur ce dépôt est vérifié **sous Windows** par
[GitHub Actions](.github/workflows/ci.yml) : formatage, `clippy -D warnings`
(`unwrap()` et `expect()` interdits hors tests), plus de mille tests, la
documentation, les licences des dépendances (GPLv3-compatibles) et les règles
d'architecture.

Beaucoup de tests sont des **oracles** : ils comparent ce que produit
l'application à ce que produisait la version Python d'origine sur une vraie
collection (`tests/fixtures/`).

### Organisation du code

| Crate | Rôle |
|---|---|
| `ygo-core` | règles métier pures — aucune entrée/sortie |
| `ygo-db` | SQLite, schémas extraits des bases réelles |
| `ygo-sources` | YGOJSON, YGOPRODeck, Yugipedia — client HTTP à quotas, cache disque |
| `ygo-images` | téléchargement, file d'attente, cache d'images |
| `ygo-app` | orchestration — ne connaît aucun type d'interface |
| `ygo-ui` | l'interface (egui / eframe) |
| `ygo-cli` | la ligne de commande |

`cargo doc --open` ouvre la documentation complète : la plupart des décisions
de conception y sont expliquées à l'endroit du code où elles s'appliquent.

## Licence

[GNU GPL v3](LICENSE) — vous pouvez utiliser, étudier, modifier et redistribuer
ce programme, à condition que les versions redistribuées restent sous la même
licence.
