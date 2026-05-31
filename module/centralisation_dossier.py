# Yu-Gi-Oh! Collection Manager
# Copyright (C) 2026  Althalusse
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

import sys
import os


def get_exe_dir():
    """
    Retourne le dossier où se trouve l'exécutable ou le script principal.
    Utilise sys.executable (et non sys.argv[0]) pour être robuste en mode
    administrateur sous Windows — sys.argv[0] peut pointer vers un chemin
    UAC temporaire quand l'appli est lancée avec élévation de droits.
    """
    if getattr(sys, 'frozen', False):
        # Compilé .exe PyInstaller : sys.executable = chemin réel du .exe
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


ROOT_FOLDER = get_exe_dir()
BDD_FOLDER = os.path.join(ROOT_FOLDER, "bdd")
IMG_FOLDER = os.path.join(ROOT_FOLDER, "img")
MODULE_FOLDER = os.path.join(ROOT_FOLDER, "module")
EXPORT_FOLDER = os.path.join(ROOT_FOLDER, "export")
DEFAULT_IMAGE_PATH = os.path.join(IMG_FOLDER, "notfound.jpg")
MODULE_IMG_FOLDER = os.path.join(MODULE_FOLDER, "img")

CLASSEUR_FOLDER     = os.path.join(BDD_FOLDER, "classeur_creer")
CARDINFO_DB         = os.path.join(BDD_FOLDER, "cardinfo.db")
IMAGES_SMALL_FOLDER = os.path.join(IMG_FOLDER, "small")   # pool partagé §5
IMAGES_BOOSTERS_FOLDER = os.path.join(IMG_FOLDER, "boosters")   # images de boosters (couvertures classeurs)
AFFICHER_CARTE = CLASSEUR_FOLDER
CONFIG_FILE = os.path.join(BDD_FOLDER, "rarity_config.json")
APP_CONFIG_FILE = os.path.join(BDD_FOLDER, "app_config.json")
DB_PATH = CARDINFO_DB
BACKUP_DIR = os.path.join(ROOT_FOLDER, "backups")
LAST_UPDATE_FILE = os.path.join(BDD_FOLDER, "last_update.txt")
FIRST_RUN_FILE = os.path.join(ROOT_FOLDER, "first_run.flag")
STATS_CACHE_FILE = os.path.join(BDD_FOLDER, "stats_cache.json")
FR_NAMES_CACHE   = os.path.join(BDD_FOLDER, "fr_names_cache.json")
ANOMALIES_FILE = os.path.join(BDD_FOLDER, "anomalies.json")

PATHS = {
    "root": ROOT_FOLDER,
    "bdd": BDD_FOLDER,
    "export": EXPORT_FOLDER,
    "img": IMG_FOLDER,
    "classeur": CLASSEUR_FOLDER,
    "stats_cache": STATS_CACHE_FILE,
}


def get_paths():
    return PATHS


def init_folders():
    """
    ✅ CORRIGÉ : création des dossiers déplacée ici, hors du niveau module.
    Appelée explicitement depuis main.py au démarrage.
    Compatible .exe (PyInstaller/cx_Freeze) : ROOT_FOLDER pointe toujours
    vers le dossier du .exe, donc les dossiers sont créés au bon endroit.
    """
    os.makedirs(CLASSEUR_FOLDER,        exist_ok=True)
    os.makedirs(IMG_FOLDER,             exist_ok=True)
    os.makedirs(IMAGES_SMALL_FOLDER,    exist_ok=True)
    os.makedirs(IMAGES_BOOSTERS_FOLDER, exist_ok=True)
    os.makedirs(EXPORT_FOLDER,          exist_ok=True)
    os.makedirs(BDD_FOLDER,             exist_ok=True)
    os.makedirs(BACKUP_DIR,             exist_ok=True)

from contextlib import contextmanager

@contextmanager
def sqlite_ctx(db_path):
    """
    Context manager SQLite qui garantit conn.close() même sous Windows.
    Remplace 'with sqlite3.connect(path) as conn' partout dans le projet.

    PRAGMA appliqués à chaque connexion (juste après l'ouverture, hors
    transaction) :
      - busy_timeout=5000 : attend jusqu'à 5 s qu'un verrou se libère au lieu
        d'échouer aussitôt (lecture UI + écriture du worker de téléchargement
        en parallèle). Python pose déjà ce délai via timeout=5.0 ; on le rend
        explicite.
      - journal_mode=WAL : lecteurs et écrivain ne se bloquent plus
        mutuellement → fin des « database is locked » lors d'un rafraîchissement
        de l'UI pendant un téléchargement. La conversion n'a lieu qu'à la
        première ouverture de chaque base (no-op ensuite). cardinfo.db était
        déjà en WAL ; on aligne les bases de classeur.
      - synchronous=NORMAL : combiné recommandé avec WAL (préserve toujours
        l'intégrité ; ne risque de perdre que les toutes dernières écritures
        en cas de coupure d'alimentation) — écritures plus rapides.

    Sûr vis-à-vis de l'export/backup : l'export produit du CSV via SQLite et le
    backup utilise l'API src.backup(dst), tous deux cohérents avec WAL (aucune
    copie brute du fichier .db ailleurs dans le projet).

    Usage:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            ...
    """
    import sqlite3
    conn = sqlite3.connect(db_path, timeout=5.0)
    try:
        # PRAGMA hors transaction (juste après connect). Protégés : sur un
        # support en lecture seule ou un partage réseau, WAL peut être refusé
        # sans que ce soit bloquant — on continue alors avec le mode par défaut.
        try:
            conn.execute("PRAGMA busy_timeout=5000")
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=NORMAL")
        except Exception:
            pass
        yield conn
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()
