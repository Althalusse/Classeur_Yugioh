# Yu-Gi-Oh! Collection Manager

Application de bureau (Windows) pour cataloguer, organiser et suivre une collection de cartes **Yu-Gi-Oh!**, classeur par classeur (set par set), avec images, raretés officielles, statistiques de complétion et import/export de collection.

> Interface bilingue **FR / EN**. Données issues de YGOPRODeck, Yugipedia et YGOJSON.

---

## ⚠️ Disclaimer

Il s'agit d'une application **« vibecodée »**. Je l'ai faite **pour moi** à l'origine, afin de simplifier le tri de mes collections de cartes dans les classeurs selon les **raretés** et leur **code set** (par exemple `RA02-FR001`). L'idée : visualiser ma collection sans avoir à tout compter à la main et me prendre la tête.

Je la partage au cas où elle servirait à d'autres, mais elle reste un projet personnel, sans garantie ni support.

<!-- Remplacez par vos propres captures -->
<!-- ![Accueil](docs/screenshot_accueil.png) -->
<!-- ![Classeur](docs/screenshot_classeur.png) -->

---

## But

Yu-Gi-Oh! Collection Manager permet de reconstituer numériquement sa collection physique : on crée un classeur pour chaque set (booster, structure deck, tin, collection…), l'application récupère automatiquement la liste complète des cartes du set avec leurs raretés et leurs images, puis on coche les cartes possédées. L'outil calcule en temps réel le taux de complétion par classeur et global, et permet d'échanger sa collection via un format CSV standard.

---

## Fonctionnalités

### Gestion des classeurs
- **Création automatique par set** : saisie d'un code de set → récupération de toutes les cartes (EN + FR) avec raretés et images via l'API YGOPRODeck, avec repli sur une base locale (`cardinfo.db`).
- **Vue grille** des cartes d'un classeur, grille configurable (colonnes × lignes, globale ou par classeur).
- **Cover automatique** du classeur : image officielle du booster/deck (YGOPRODeck, avec repli **Yugipedia** quand l'image manque).
- **Recherche** de classeurs par nom ou code (filtrage instantané, utile sur de grandes collections).

### Suivi de la collection
- Marquage des cartes **possédées** avec **quantité**, **qualité** (Near Mint, etc.) et **édition**.
- **Statistiques par classeur** : répartition des cartes possédées **par rareté** (panneau dépliable).
- **Statistiques globales** : vue d'ensemble de tous les classeurs avec progression et détail par rareté.

### Raretés
- Table de référence centralisée des raretés (convention **Scanflip**).
- Tri des cartes par **priorité de rareté** personnalisable.
- Affichage configurable du nombre de raretés par artwork.
- **Correction automatique via Yugipedia** des raretés erronées renvoyées par YGOPRODeck (ex. annotation « New artwork » rétablie en sa vraie rareté), avec préservation de la possession.

### Cartes spéciales
- **Anomalies d'artwork** : détection et correction des cartes possédant plusieurs artworks (sélection multiple, menu contextuel).
- **Artworks alternatifs** et prints « extended art » (Overframe) complétés via Yugipedia.
- **Ajout manuel** de cartes au classeur (avec garde-fou sur les raretés inconnues).

### Import / Export
- **Export** de la collection au format CSV **Scanflip** (UTF-8 BOM, round-trip exact).
- **Import** d'une collection au même format.

### Données & maintenance
- **Mise à jour de la base** (cardinfo.db) depuis YGOPRODeck + Yugipedia, déclenchée uniquement si la version distante a changé.
- Outils de maintenance : initialisation de la base, purge du cache d'images, réparation des set_codes/raretés, correction des raretés et récupération des covers manquantes via Yugipedia.

### Personnalisation (Options)
- **Langue** de l'interface et des noms de cartes (FR / EN).
- **Taille de police** ajustable.
- **Source des images** : YGOPRODeck (JPEG HD) ou Yugipedia (PNG par print).
- **Redémarrage assisté** : lorsqu'un réglage nécessite un redémarrage (langue, police), un bouton relance l'application automatiquement.

### Divers
- **Centre d'activité** : suivi des téléchargements en arrière-plan.
- **Page Merci** : crédits des sources de données utilisées.
- Téléchargements et tâches lourdes exécutés en arrière-plan (interface non bloquante).

---

## Stack technique

| Composant | Technologie |
|---|---|
| Langage | Python 3 |
| Interface | CustomTkinter |
| Base de données locale | SQLite |
| Images | Pillow (PIL) |
| Réseau | requests |
| Fenêtre intégrée (Ko-fi) | pywebview |
| Packaging | PyInstaller (exécutable Windows `--onefile`) |

---

## Installation (développement)

Prérequis : **Python 3.10+** sur Windows.

```bash
# 1. Cloner le dépôt
git clone https://github.com/<votre-compte>/<votre-repo>.git
cd <votre-repo>

# 2. (Recommandé) environnement virtuel
python -m venv .venv
.venv\Scripts\activate

# 3. Installer les dépendances
pip install -r requirements.txt

# 4. Lancer
python main.py
```

Au **premier lancement**, l'application initialise sa base de données interne (`cardinfo.db`) depuis YGOPRODeck — une connexion Internet est requise pour cette étape et pour la création de classeurs.

---

## Build (exécutable Windows)

Un script de build PyInstaller `--onefile` est fourni :

```bash
build_windows_onefile.bat
```

Le binaire `YGO_Collection_Manager.exe` est généré dans `dist/`.

> **Note pywebview** : la fenêtre Ko-fi intégrée utilise le runtime WebView2 (préinstallé sur Windows 10/11). En son absence, le lien Ko-fi s'ouvre dans le navigateur par défaut (repli automatique).

---

## Utilisation rapide

1. **Créer un classeur** : bouton « + Nouveau classeur », saisir le code du set (ex. `RA04`, `LOB`, `SDWD`).
2. **Cocher les cartes possédées** dans la vue grille (quantité, qualité, édition).
3. **Suivre la complétion** via le panneau de statistiques (par classeur ou global).
4. **Exporter / importer** sa collection au format CSV depuis l'onglet Import/Export.
5. **Ajuster** l'interface dans Options (langue, taille de police, source d'images).

---

## Structure du projet

```
main.py                     Point d'entrée
build_windows_onefile.bat   Script de build PyInstaller
requirements.txt            Dépendances Python
kofi_viewer.py              Fenêtre Ko-fi (pywebview)
module/
├─ ui/                      Écrans (accueil, classeur, options, stats, etc.)
├─ creation_classeur/       Création de classeurs (API / base locale)
├─ carte_posseder/          Suivi des cartes possédées
├─ carte_custom/            Ajout manuel de cartes
├─ gestion_rarete/          Raretés : référence, tri, correction Yugipedia
├─ statistique/             Statistiques par classeur et globales
├─ anomalie/                Anomalies d'artwork
├─ import_csv/ · export/    Import / export CSV (format Scanflip)
├─ donnees/                 Synchronisation des données de référence
├─ img_dl/ · gestion_img/   Téléchargement et cache des images
├─ i18n/                    Traductions FR / EN
├─ config/                  Préférences
├─ utilitaire/              Migrations, redémarrage
└─ version/                 Contrôle de version de la base
```

---

## Sources de données & crédits

Cette application s'appuie sur le travail de plusieurs projets et communautés :

| Source | Usage | Lien |
|---|---|---|
| **YGOPRODeck** | Base de données des cartes, sets et images HD | https://ygoprodeck.com |
| **Yugipedia** | Raretés officielles, images de cover, artworks | https://yugipedia.com |
| **YGOJSON** | Données de cartes structurées (open source) | https://github.com/iconmaster5326/YGOJSON |
| **Scanflip** | Convention de format pour l'import/export de collection | https://scanflip.fr |

Merci à leurs équipes et à leurs contributeurs.

---

## Soutien

**Plutôt que de me soutenir, soutenez les projets et communautés sans lesquels cette application n'existerait pas.** Ce sont eux qui font tout le travail de fond — je n'ai fait que les assembler.

- **YGOPRODeck** — base de données des cartes, sets et images — https://ygoprodeck.com
- **Yugipedia** — raretés officielles, covers et artworks — https://yugipedia.com
- **YGOJSON** — données de cartes structurées, open source — https://github.com/iconmaster5326/YGOJSON

Un grand merci à leurs équipes et à leurs contributeurs.

---

## Licence

Ce projet est distribué sous licence **GNU General Public License v3.0** (GPL-3.0). Voir le fichier [`LICENSE`](LICENSE) pour le texte complet.

En résumé : vous êtes libre d'utiliser, étudier, modifier et redistribuer ce logiciel, à condition que tout travail dérivé soit lui aussi publié sous GPL-3.0 et que le code source reste accessible.

Copyright © 2026 Althalusse.

---

## Avertissement

Projet **non officiel**, sans affiliation avec Konami ni avec les sources de données citées. *Yu-Gi-Oh!* est une marque déposée de Konami. Les images et données de cartes appartiennent à leurs détenteurs de droits respectifs et sont utilisées ici à des fins de gestion de collection personnelle.
