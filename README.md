# ygo-binder — Yu-Gi-Oh! Collection Manager V2 (Rust)

Portage Rust de la V1.0.4 Python. Objectif : **stabilité** — supprimer les
violations d'accès natives que la discipline applicative Python ne peut pas
empêcher.

Documents de référence, à la racine du laboratoire :

- `Cahier_des_Charges_Migration_Rust.md` — contrat de migration, invariants
- `Plan_Optimisation_Migration_Rust.md` — ordre de portage, oracle, choix d'interface
- `Construction_Projet_YuGiOh_Binder_RUST.md` — arborescence, dépendances, rôle des modules

---

## État

| Crate | Contenu | État |
|---|---|---|
| `ygo-core` | chemins, configuration, raretés, source d'images, version, journal, modèles, **tri** | **fait** |
| `ygo-db` | schémas, migrations, connexions sérialisées, construction de `cardinfo.db` | **fait** |
| `ygo-sources` | client HTTP à quotas, YGOJSON (archive streamée), YGOPRODeck, version, **Yugipedia : Set lists, structure et artworks** | **fait** |
| `ygo-app` | initialisation, Overframe, **création complète d'un classeur**, passe artworks, données de l'accueil | **fait** |
| `ygo-cli` | diagnostic : `schema`, `config`, `creer-vide`, `init`, `classeur`, **`creer`**, `overframe`, `artworks`, `comparer` | **fait** |
| `ygo-images` | téléchargement, file d'attente, cache | ✔ |
| `ygo-app` (suite) | import/export Scanflip, artworks alternatifs, anomalies, statistiques | à venir |
| `ygo-ui` | interface — **egui / eframe 0.36**, tranché sur mesure le 2026-08-26 | à venir |

353 tests, `clippy -D warnings` vert, `cargo fmt --check` vert, `cargo doc` sans avertissement.

---

## Compiler et tester

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

> **Les artefacts ne vont pas dans `target/`.** `.cargo/config.toml` les
> renvoie dans `../../Projet Rust COMPIL`, pour garder le dossier des sources
> propre — un `target/` complet pèse plus d'un gigaoctet. Rien à faire :
> n'importe quelle commande `cargo` lancée depuis le projet suit ce réglage.
> Le binaire se trouve donc dans
> `Projet Rust COMPIL\release\ygo-cli.exe`.

## Construire la base de référence

```bash
# Pipeline complet : archive YGOJSON + catalogue YGOPRODeck
ygo-cli init ./essai

# Hors ligne, depuis une archive déjà téléchargée
ygo-cli init ./essai --archive ./aggregate.zip --catalogue ./cardinfo.json

# Comparer le résultat à la base produite par la V1.0.4
ygo-cli comparer "../Projet Python/V1.0.4/bdd/cardinfo.db" ./essai/bdd/cardinfo.db
```

Mesuré sur une exécution réelle, réseau compris :

| | valeur |
|---|---|
| Durée totale, téléchargement compris | **13 s** |
| Archive téléchargée | 33 Mo, **sur disque, jamais en mémoire** |
| Mémoire résidente maximale | **446 Mo** |
| Base produite | 162 Mo |

Comptages obtenus, face à la base réelle produite par la V1.0.4 :

| table | V1.0.4 | Rust | |
|---|---|---|---|
| `sets` | 3 306 | 3 306 | = |
| `set_locales` | 7 051 | 7 051 | = |
| `cards` | 14 616 | 14 616 | = |
| `card_texts` | 118 360 | 118 360 | = |
| `card_images` | 16 121 | 16 121 | = |
| `cards_missing_fr` | 742 | 742 | = |
| `set_prints` | 326 450 | **326 414** | −36 |

**Les sept tables du schéma d'initialisation concordent.** L'écart de 36 lignes
sur `set_prints` ne vient pas de l'initialisation : ce sont les tirages ajoutés
**après**, par la passe Overframe, qui n'est pas encore portée. Trois
vérifications indépendantes le confirment :

- le `cardinfo.db` de la V1.0.3, qui n'a pas subi cette passe, porte
  exactement **326 414** lignes ;
- le journal de la V1.0.4 dit `[Overframe] LOCR-JP : +36 prints, 18 corriges` ;
- la table de suivi `overframe_sync` est peuplée côté V1.0.4, absente côté Rust.

`ygo-cli comparer` reconnaît ce cas, l'explique, et ne le compte pas comme une
divergence — mais uniquement si c'est le seul écart, sans quoi il masquerait un
vrai problème derrière une explication commode.

## Combler les 36 tirages : la passe Overframe

```bash
# Sur une COPIE — la commande écrit dans la base indiquée.
ygo-cli overframe ./essai/bdd/cardinfo.db

# Pour voir sans écrire : tout est déroulé, puis annulé.
ygo-cli overframe ./essai/bdd/cardinfo.db --simuler
```

YGOJSON est incomplète sur les sets à traitement Overframe : pour une carte
chase, elle ne liste que les tirages de base, et ne distingue pas le cadre. Or
l'application place les cartes séquentiellement — un tirage manquant ne fait pas
un trou, il **décale toute la grille**.

La passe lit la Set list Yugipedia, en déduit les tirages *extended art*, et
réconcilie. Elle n'efface jamais rien : selon le cas elle ajoute la ligne
Overframe en gardant la normale, ou corrige un drapeau que YGOJSON avait posé à
tort.

La chaîne entière est éprouvée sur les données réelles, sans raccourci :

```text
wikitext (9 931 octets)  →  parse_set_list  →  classer  →  reconcilier
       rev 5945579              98 entrées     264 + 54      282 → 318
```

```bash
cargo test -p ygo-sources --test oracle_setlist    # parser Rust == parser Python
cargo test -p ygo-app --test oracle_overframe      # 282 lignes → 318, à l'identique
```

Le second exige les 318 lignes **colonne par colonne et identifiant par
identifiant**. Les trois branches y sont exercées : 18 Grand Master Rare
ajoutées sans URL (aucune ligne d'où hériter), 18 Ultra Rare ajoutées avec l'URL
héritée et la normale conservée, 18 Prismatic Secret Rare corrigées sans qu'une
seule ligne soit créée.

## Vérifier une installation Python existante

`ygo-cli` lit une installation V1.0.3 ou V1.0.4 **en lecture seule** et compare
ses schémas SQLite aux DDL de référence.

```bash
# Schémas de cardinfo.db et de tous les classeurs
ygo-cli schema "../Projet Python/V1.0.3"

# Préférences, telles que le Rust les interprète
ygo-cli config "../Projet Python/V1.0.3"

# Arborescence neuve avec une cardinfo.db vide mais conforme
ygo-cli creer-vide "C:/tmp/essai"
```

Résultat attendu sur l'installation V1.0.3 : 17 classeurs, 4 453 cartes,
742 possédées, tous les schémas conformes.

## Le tri, comparé au Python sur les classeurs réels

`ygo-core::tri` porte `tri_carte.py`. Comme le projet Python n'a aucun test, la
vérité terrain a été **produite** : `outils/oracle_tri.py` importe le vrai
module, neutralise ses deux dépendances par injection dans `sys.modules`, rejoue
la requête exacte de l'écran classeur sur une copie des bases, et fige entrée et
sortie.

| | |
|---|---|
| Classeurs couverts | **26** — 17 de la V1.0.3, 9 de la V1.0.4 |
| Cartes | **7 136** |
| Ordres de critères comparés | les **six** permutations, soit 156 comparaisons |
| Filtre N raretés | `n` = 1, 2, 3, 5 |

```bash
cargo test -p ygo-core --test oracle_tri
```

L'échec affiche la première position divergente et les deux cartes qui s'y
trouvent — pas deux listes de plusieurs centaines d'entiers.

## Les artworks que nulle API ne décrit

Certains sets réimpriment une carte avec une **illustration différente** sans
créer d'entrée côté YGOPRODeck. Yugipedia, lui, héberge les fichiers, sous une
convention de nommage :

```text
VanquishSoulRazen-RA05-EN-UR-1E.png        une illustration du tirage
VanquishSoulRazen-RA05-EN-UR-1E-AA.png     la seconde, dite « AA »
```

Le piège est que **le suffixe ne dit pas « ceci est une variante »** : Yugipedia
ne l'ajoute que pour désambiguïser deux fichiers homonymes. `RA05-EN083` Dark
Magician *est* du stamp artwork et n'a pourtant aucun suffixe. L'autorité est la
Set list, qui sépare `== Main pool ==` de `== Variant art pool ==`.

```bash
cargo test -p ygo-sources --test oracle_artwork
```

L'oracle ne demande **aucune requête** : la table `card_images_externes` de
l'installation réelle porte **1 613 noms de fichiers**, chacun déjà découpé par
le code Python. Neuf sets, deux langues, quatre éditions, quatorze formes de
rareté — dont une rareté `2`, parce qu'un nom de fichier réel porte un jeton
numérique là où le format attend une abréviation.

## Créer un classeur sans une seule requête

L'application construit toujours le classeur depuis `cardinfo.db` d'abord.
La décision la plus délicate y est le choix de la **locale d'énumération** :
énumérer en français évite les doublons d'artworks que YGOJSON traîne sur les
cartes-icônes, mais sur un set récent la traduction peut n'être que partielle.
Une condition « français non vide » a coûté six raretés sur sept à `RA05` en
avril 2026. La règle est donc : le français n'est retenu **que s'il est au
moins aussi complet** que l'anglais.

```bash
cargo test -p ygo-app --test oracle_creation
```

L'oracle rejoue le tri et l'heuristique sur **26 classeurs et 5 488 lignes**,
figés depuis le vrai code Python. Le SQL, lui, est éprouvé par des bases
miniatures bâties sur le vrai DDL — une par branche, dont les trois bugs de
production corrigés en 2026 : `RA05`, `SDWD`, et le classeur japonais.

Sur vos propres données :

```bash
ygo-cli classeur "../Projet Python/V1.0.3" RA02
ygo-cli classeur "../Projet Python/V1.0.3" RA05 --ecrire ./essai_RA05.db
```

La commande dit combien de lignes le chemin local produit, la moyenne de
raretés par carte, et ce que l'application déciderait — écrire depuis le local,
ou basculer sur YGOPRODeck.

Le repli, justement, a son propre oracle — celui-là porte **entrée et sortie**,
et compare les 1 572 lignes de cinq sets champ par champ :

```bash
cargo test -p ygo-app --test oracle_api
```

Sur `RA05`, il chiffre ce que le repli apporte : **692 lignes contre 228** en
local. C'est la justification de toute la cascade de décision.

### Créer un classeur, de bout en bout

```bash
ygo-cli creer <installation> RA02        # cascade, écriture, puis artworks
ygo-cli creer <installation> RA02 --sans-artworks   # sans réseau Yugipedia
```

C'est le `create_classeur` du Python : court-circuit si le classeur est déjà
peuplé, purge d'un dossier résiduel, cascade `cardinfo.db` → YGOPRODeck,
écriture, puis passe artworks.

Deux choses que le portage fait autrement, et volontairement.

**L'échec de la passe artworks est visible.** Le Python l'absorbe dans un
`try/except` qui journalise ; ici il devient `Artworks::Ignoree(raison)`. Un
classeur créé sans ses variantes n'est pas la même chose qu'un classeur créé
avec, et l'appelant a le droit de le savoir — la création, elle, reste
protégée exactement comme avant.

**La cascade est arbitrée par une fonction pure.** `creer` interroge le réseau,
puis `arbitrer` choisit — c'est le même découpage que `planifier` / `appliquer`
dans la passe artworks, et il a la même raison d'être : sans lui, le repli
« API injoignable → écrire le local imparfait » serait intestable sans couper
Internet. La mutation qui l'a montré survivait à toute la suite.

Trois étapes du Python ne sont pas encore portées — correction des raretés via
Yugipedia, migration des set_codes, overrides d'anomalies. Toutes trois sont
des greffons que le Python enveloppe déjà dans un `try/except` : ne rien
brancher revient au chemin d'exception qu'il prévoit. `Greffons` marque
l'emplacement plutôt que de laisser le trou invisible.

### La passe artworks

La Set list Yugipedia fait autorité sur la **structure** — quels numéros,
quelles raretés, lesquels sont en variante d'illustration. Les API gardent les
métadonnées et les illustrations normales. Yugipedia ne fournit que les images
des tirages **en variante**.

```bash
cargo test -p ygo-app --test oracle_artworks   # 30 illustrations sur 318 lignes
cargo test -p ygo-app --lib artworks           # 28 tests, 17 mutations tuées
ygo-cli artworks <classeur.db> --simuler       # la passe entière, sans rien écrire
```

L'oracle n'a demandé **aucune requête** : la Set list était capturée, le
classeur de la V1.0.3 est l'état d'avant, et les 302 images sont dans
`card_images_externes`. `outils/oracle_artworks.py` rejoue le vrai
`completer_artworks_variantes` sur une copie et fige le diff.

Le piège du module tient en une phrase : Yugipedia n'ajoute `-AA` ou `-EA` à un
nom de fichier que pour **désambiguïser** deux homonymes, si bien qu'un tirage
qui n'existe qu'en variante porte un nom parfaitement ordinaire. Le prendre pour
l'illustration d'origine fait manquer `RA05-EN083` ; prendre l'inverse pose sur
la ligne variante la même image que sur la normale — invisible dans le code,
criant à l'écran.

Cet oracle est un bon ancrage et un mauvais discriminateur : sur dix mutations
du planificateur, deux seulement le font tomber. Ce sont les 28 tests unitaires
du module qui tiennent les branches, et la table mutation → test qui les tue est
dans l'en-tête de `artworks.rs`.

L'oracle porte aussi l'**écriture** : le classeur est reconstruit depuis l'état
d'avant, la passe tourne, et les 318 lignes sont comparées colonne par colonne à
celles du Python — les 288 qu'il ne faut pas toucher comprises. Le rejouer une
seconde fois ne change rien : l'`UPDATE` porte une garde
`card_image_uuid NOT LIKE 'yugipedia:%'`, et le compte tombe alors à 0 alors que
le plan, lui, prévoit toujours ses 30 illustrations. C'est pourquoi
l'applicateur rend son propre bilan plutôt que celui du planificateur.

### L'accueil, et la moitié d'un écran qu'on peut encore prouver

C'est le premier lot dont une part échappe à l'oracle : on ne fige pas des
pixels. La parade est de faire passer la frontière au bon endroit — tout ce que
l'écran **affiche** est calculé dans `ygo-app::accueil`, hors de toute
interface, et se compare champ par champ à ce que le Python produit sur les
**26 classeurs réels** des deux installations.

```bash
cargo test -p ygo-app --test oracle_accueil   # 26 classeurs, deux installations
cargo test -p ygo-app --lib accueil           # 18 tests, dont les 3 pistes que l'oracle n'atteint pas
```

Deux tests visent le disque réel de l'utilisateur et se **sautent** ailleurs.
Un test qui se saute passe à vide : c'est pourquoi deux autres rebâtissent une
installation miniature depuis la fixture, sur le vrai DDL de classeur, et
tournent partout.

Ce que cet oracle ne prouve pas, mesuré plutôt que supposé : les 26 classeurs
ont **tous** une cover de booster et **tous** une grille 3×3 — soit exactement
la valeur par défaut. Les trois autres pistes de couverture et la lecture de la
table `meta` ne sont donc jamais exercées par les données réelles. Les tests
unitaires s'en chargent, et les mutations qui les visent tombent.

### Les deux prototypes d'interface

`prototypes/accueil-egui` et `prototypes/accueil-iced` affichent **le même
écran** en consommant **le même** `ygo_app::accueil` : pas une règle métier
n'est dupliquée, seuls les pixels changent. C'est ce qui rend la comparaison
honnête — on mesure les frameworks, pas deux façons de compter des cartes.

```bash
cd prototypes/accueil-egui && cargo run --release -- "../../../Projet Python/V1.0.3"
cd prototypes/accueil-iced && cargo run --release -- "../../../Projet Python/V1.0.3"
```

Ils sont **hors du workspace**, volontairement : ce sont des artefacts
d'arbitrage, l'un des deux sera jeté, et les inclure ferait compiler deux piles
graphiques à chaque `cargo test --workspace`. Chacun a aussi son propre
répertoire de compilation — sans quoi un `cargo clean` dans l'un vide celui des
autres, ce qui a été constaté à la dure.

#### Le verdict : egui

Mesuré sur le poste de l'utilisateur, avec ses dix-sept classeurs réels :

| critère | egui | Iced |
|---|---|---|
| premier rendu | **293 ms** | 1 194 ms |
| binaire release | **16,1 Mo** | 27,7 Mo |
| crates dans l'arbre | **379** | 549 |
| compilation propre | **1 min 09** | 1 min 38 |

egui devance sur tous les axes, rendu compris. La lecture des données est
identique aux deux (126 ms contre 134) — c'est le même `ygo-app::accueil` en
dessous, et c'était tout l'intérêt du montage.

Trois bugs de prototype ont dû être corrigés **avant** de conclure, tous de mon
fait : des URI `file://` malformées sous Windows, un mauvais usage d'`egui::Grid`,
et — le plus grave — une grille Iced figée à quatre colonnes sur la foi d'un
commentaire affirmant qu'Iced n'a pas de grille fluide. Il en a une. J'allais
départager deux frameworks en reprochant à l'un une limitation inexistante.

Cet arbitrage ne prouve pas tout : l'accueil est le plus facile des douze
écrans. Le premier écran du vrai `ygo-ui` sera donc `ecran_classeur` — 1 120
cartes, images de 314 Ko, défilement —, et le prototype Iced reste sur le disque
tant qu'il n'est pas passé.

---

## Deux principes qui structurent le code

**1. Ce qui peut être extrait ne s'écrit pas.**
`assets/schema/*.sql` provient de `sqlite3 <base> .schema` sur les bases réelles.
Aucune transcription manuelle. Ça a déjà servi : l'Annexe A du cahier des charges
décrit 8 tables et 9 index pour `cardinfo.db`, la base réelle en a **11 et 10** —
`anomalies`, `overframe_sync` et `card_images_externes` y manquent.

Même principe pour `assets/raretes_reference.json` : les 44 codes Scanflip sont
**sérialisés depuis le module Python**, dans leur ordre d'insertion. L'ordre
n'est pas cosmétique — quatorze clés normalisées sont en collision, et le Python
laisse la dernière écraser les précédentes.

**2. Le portage est iso-fonctionnel.**

Les comportements douteux du Python sont **reproduits**, pas corrigés — sans quoi
on ne saurait plus distinguer un portage fautif d'une amélioration voulue. Deux
exemples déjà encodés et testés :

- une rareté inconnue vaut `9999` au tri et `0` au filtrage — deux défauts
  divergents dans le même fichier Python ;
- `PLatinum Secret Rare` (80 lignes en base) n'existe pas dans
  `rarity_config.json` et reste donc inconnue ;
- une image externe (Yugipedia) a un `card_image_id` **négatif** — le CRC32 de
  son nom de fichier. Yugipedia hébergeant un fichier par rareté, la même
  illustration reçoit plusieurs identifiants ; leur donner des rangs d'artwork
  distincts les classerait dans l'ordre du **hasard des hachages**, avant que la
  rareté ne soit consultée. Elles partagent donc un rang unique ;
- les **clés étrangères sont désactivées** : les données YGOJSON contiennent de
  vraies violations d'intégrité, que la V1.0.4 insère sans broncher. `rusqlite`
  en `bundled` les applique par défaut, contrairement au module `sqlite3` de
  Python — sans le PRAGMA, l'initialisation échoue. Les filtrer donnerait une
  base plus propre mais **différente**, donc invérifiable.

Ces écarts sont au backlog, à traiter après la bascule.


---

## Soutien

**Privilégiez les projets et communautés sans lesquels cette application
n'existerait pas** — ce sont eux qui rendent tout cela possible, je n'ai fait
que les assembler :

- **YGOJSON** — cartes, sets et tirages — https://github.com/iconmaster5326/YGOJSON
- **YGOPRODeck** — base de données des cartes, sets et images — https://ygoprodeck.com
- **Yugipedia** — raretés officielles, images des tirages — https://yugipedia.com

Un grand merci à leurs équipes et à leurs contributeurs.

Cela dit, si le cœur vous en dit, vous pouvez toujours m'offrir un café :

☕ **Ko-fi** : https://ko-fi.com/althalusse


---

## Licence

GNU General Public License v3.0 — comme la V1. Voir le fichier `LICENSE` du
projet Python. `cargo deny check licenses` vérifie en CI que toutes les
dépendances restent compatibles.
