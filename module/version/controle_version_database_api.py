"""
Module pour contrôler la version de la base de données distante (API YGOProDeck)
et gérer le fichier last_update.txt.
"""

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

import os
import json
import sqlite3
import requests
from datetime import datetime

from module.centralisation_dossier import DB_PATH, BACKUP_DIR, LAST_UPDATE_FILE
from module import BDD_creation


def check_db_version():
    """
    Récupère les informations de version distante via l'API YGOProDeck.
    Retourne le JSON de version (list ou dict).
    """
    url = "https://db.ygoprodeck.com/api/v7/checkDBVer.php"
    response = requests.get(url, timeout=10)
    response.raise_for_status()
    return response.json()


def check_for_updates():
    """
    Vérifie si une mise à jour est disponible.
    Retourne (True/False, infos de version ou erreur).
    """
    try:
        remote_info = check_db_version()
        local_info  = load_last_update_file(LAST_UPDATE_FILE)

        remote_version = remote_info[0]["database_version"] if isinstance(remote_info, list) else None
        local_version  = local_info[0]["database_version"]  if local_info and isinstance(local_info, list) else None

        if remote_version is None:
            return False, {"error": "Version distante non disponible"}

        return (local_version != remote_version), {
            "local":  local_version or "Aucune",
            "remote": remote_version,
        }
    except Exception as e:
        return False, {"error": str(e)}


def save_last_update_file(json_data, last_update_file_path):
    """Sauvegarde le JSON de version (toujours sous forme de liste)."""
    data_to_save = json_data if isinstance(json_data, list) else [json_data]
    with open(last_update_file_path, "w", encoding="utf-8") as f:
        json.dump(data_to_save, f, indent=2, ensure_ascii=False)


def load_last_update_file(last_update_file_path):
    """Charge last_update.txt. Retourne None si absent ou corrompu."""
    if not os.path.exists(last_update_file_path):
        return None
    try:
        with open(last_update_file_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, list) else [data]
    except json.JSONDecodeError:
        os.remove(last_update_file_path)
        return None
    except Exception:
        return None


def create_backup():
    """
    Sauvegarde cardinfo.db via l'API sqlite3 (VACUUM INTO) pour éviter
    de copier un fichier potentiellement verrouillé par une connexion ouverte.
    """
    try:
        if not os.path.exists(DB_PATH):
            return True  # Rien à sauvegarder

        os.makedirs(BACKUP_DIR, exist_ok=True)
        timestamp   = datetime.now().strftime("%Y%m%d_%H%M%S")
        backup_path = os.path.join(BACKUP_DIR, f"yugioh_cards_backup_{timestamp}.db")

        with sqlite3.connect(DB_PATH) as src, sqlite3.connect(backup_path) as dst:
            src.backup(dst)
        return True
    except Exception:
        return False


def update_database(log=None):
    """
    Met à jour la base de données avec les dernières données YGOPRODeck + YGOJSON.
    Délègue l'intégralité du flux data à BDD_creation.run_init(log).
    Cette fonction est responsable uniquement de :
      1. La sauvegarde préalable de cardinfo.db
      2. La mise à jour du fichier de version après succès

    log : callable(message: str, couleur: str) — optionnel, pour l'UI.
    Retourne (bool, str).
    """
    def _log(msg, couleur="blue"):
        if log:
            log(msg, couleur)

    try:
        _log("Sauvegarde de la base de données actuelle...", "blue")
        if not create_backup():
            return False, "Échec de la création de la sauvegarde"
        _log("✅ Sauvegarde créée.", "green")

        success = BDD_creation.run_init(log=log if log else lambda m, c: None)
        if not success:
            return False, "Échec de la mise à jour de la base de données."

        remote_info = check_db_version()
        save_last_update_file(remote_info, LAST_UPDATE_FILE)
        _log("✅ Base de données mise à jour avec succès.", "green")

        return True, "Base de données mise à jour avec succès."

    except Exception as e:
        return False, f"Erreur lors de la mise à jour : {str(e)}"
