import os
import sqlite3
from module.centralisation_dossier import CLASSEUR_FOLDER
from module.gestion_rarete.tri_carte import sort_cartes

CARTES_PAR_PAGE = 9  # À adapter si besoin

def rechercher_carte(nom_carte=None, code_set=None):
    """
    Recherche une carte par nom ou code_set dans tous les classeurs.
    Retourne une liste de tuples : (classeur, nom_carte, code_set, index, page)
    """
    resultats = []
    for classeur in os.listdir(CLASSEUR_FOLDER):
        db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
        if not os.path.exists(db_path):
            continue
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        # Récupère toutes les cartes comme dans get_cartes_info
        cursor.execute("""
            SELECT name, card_sets_set_rarity, card_sets_set_code
            FROM cards
            WHERE card_images_image_url IS NOT NULL
        """)
        rows = cursor.fetchall()
        cartes = []
        for name, rarity, code in rows:
            cartes.append({
                "name": name,
                "rarity": rarity,
                "code": code
            })
        # Trie avec la même logique que l'affichage
        cartes_tries = sort_cartes(cartes)
        # Recherche la carte dans la liste triée
        for idx, carte in enumerate(cartes_tries):
            if (nom_carte and nom_carte.lower() in (carte["name"] or "").lower()) or \
               (code_set and code_set.lower() in (carte["code"] or "").lower()):
                page = idx // CARTES_PAR_PAGE + 1
                resultats.append((classeur, carte["name"], carte["code"], idx, page))
        conn.close()
    return resultats