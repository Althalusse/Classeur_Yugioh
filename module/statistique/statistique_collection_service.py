"""
statistique_collection_service.py — Agrégation des stats de possession par classeur.

Responsabilité unique : parcourir tous les classeurs et calculer pour chacun
le nombre de cartes totales / possédées, avec décomposition par rareté.

Note historique : une infrastructure de cache disque (stats_cache.json)
existait anciennement mais n'a jamais été utilisée par l'UI active — elle
a été retirée dans le ticket M5 pour simplifier. Si le besoin se fait
sentir à l'avenir (nombre de classeurs >> 100), un cache mémoire par
process serait plus approprié qu'un cache disque.
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
from module.centralisation_dossier import CLASSEUR_FOLDER, sqlite_ctx
from module.logger_app import log


def get_stats_collection():
    stats = []
    for nom_classeur in os.listdir(CLASSEUR_FOLDER):
        classeur_path = os.path.join(CLASSEUR_FOLDER, nom_classeur)
        db_file = os.path.join(classeur_path, f"{nom_classeur}.db")
        if not os.path.isfile(db_file):
            continue
        try:
            with sqlite_ctx(db_file) as conn:
                cursor = conn.cursor()
                cursor.execute("SELECT COUNT(*) FROM cards")
                total = cursor.fetchone()[0]
                cursor.execute("SELECT COUNT(*) FROM cards WHERE possessed = 1")
                possedees = cursor.fetchone()[0]
                cursor.execute("""
                    SELECT rarity, COUNT(*) as total,
                           SUM(CASE WHEN possessed = 1 THEN 1 ELSE 0 END) as possedees
                    FROM cards
                    WHERE rarity IS NOT NULL
                    GROUP BY rarity
                """)
                raretes = {}
                for rarity, total_rarete, possedees_rarete in cursor.fetchall():
                    if rarity:
                        raretes[rarity] = {
                            "total":      total_rarete,
                            "possedees":  possedees_rarete,
                            "pourcentage": (possedees_rarete / total_rarete * 100)
                                           if total_rarete > 0 else 0,
                        }
            stats.append({
                "nom":        nom_classeur,
                "total":      total,
                "possedees":  possedees,
                "pourcentage": (possedees / total * 100) if total > 0 else 0,
                "raretes":    raretes,
            })
        except Exception as e:
            log.info(f"Erreur avec {nom_classeur}: {e}")
    return sorted(stats, key=lambda x: x["nom"])


def stats_par_collection():
    stats = get_stats_collection()
    print(f"{'Collection':<12} | {'Possédées':<10} | {'Total':<6} | {'% Complétion':<12}")
    print("-" * 50)
    for stat in stats:
        print(f"{stat['nom']:<12} | {stat['possedees']:<10} | {stat['total']:<6} | {stat['pourcentage']:10.2f} %")


if __name__ == "__main__":
    stats_par_collection()
