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
from tkinter import messagebox
from module.centralisation_dossier import AFFICHER_CARTE, sqlite_ctx
from module.logger_app import log




def update_quantite_by_rowid(db_path: str, rowid: int, quantite: int) -> None:
    """
    Met à jour possessed + quantite pour une carte identifiée par son rowid.
    Appelé depuis gestionnaire_classeur à chaque clic +/- dans le visualiseur.
    """
    possessed = 1 if quantite > 0 else 0
    try:
        with sqlite_ctx(db_path) as conn:
            conn.execute(
                "UPDATE cards SET possessed = ?, quantite = ? WHERE rowid = ?",
                (possessed, quantite, rowid)
            )
    except Exception as e:
        log.warning(f"update_quantite_by_rowid : {e}")


def update_qualite_by_rowid(db_path: str, rowid: int, qualite: str | None) -> None:
    """
    Met à jour la qualité d'une carte identifiée par son rowid.
    Appelé depuis dialog_carte lors d'un changement de qualité.
    `qualite` vide/None est stocké comme NULL.
    """
    try:
        with sqlite_ctx(db_path) as conn:
            conn.execute(
                "UPDATE cards SET qualite = ? WHERE rowid = ?",
                (qualite or None, rowid)
            )
    except Exception as e:
        log.warning(f"update_qualite_by_rowid : {e}")


def update_quantite_in_classeur(classeur: str, name: str, set_code: str,
                                 quantite: int, rarete: str = None) -> bool:
    """
    Met à jour la quantité d'une carte possédée, identifiée par nom+code+rareté.
    Utilisé par l'inventaire (inventaire_carte_UI.py).
    Retourne True si la mise à jour a réussi.
    """
    db_path = os.path.join(AFFICHER_CARTE, classeur, f"{classeur}.db")
    if not os.path.exists(db_path):
        return False
    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            if rarete:
                cursor.execute(
                    """UPDATE cards SET quantite = ?
                       WHERE name = ? AND set_code = ?
                         AND TRIM(rarity) = TRIM(?) AND possessed = 1""",
                    (quantite, name, set_code, rarete.strip())
                )
            else:
                cursor.execute(
                    """UPDATE cards SET quantite = ?
                       WHERE name = ? AND set_code = ? AND possessed = 1""",
                    (quantite, name, set_code)
                )
        return True
    except Exception as e:
        log.warning(f"update_quantite_in_classeur : {e}")
        return False


def update_qualite_in_classeur(classeur: str, nom_carte: str, set_code: str,
                                qualite: str, rarete: str) -> bool:
    """
    Met à jour la qualité d'une carte possédée, identifiée par nom+code+rareté.
    Utilisé par l'inventaire (inventaire_carte_UI.py).
    Retourne True si la mise à jour a réussi.
    """
    db_path = os.path.join(AFFICHER_CARTE, classeur, f"{classeur}.db")
    if not os.path.exists(db_path):
        return False
    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            cursor.execute("""
                UPDATE cards SET qualite = ?
                WHERE name = ? AND set_code = ? AND rarity = ? AND possessed = 1
            """, (qualite, nom_carte, set_code, rarete))
        return True
    except Exception as e:
        log.warning(f"update_qualite_in_classeur : {e}")
        return False
