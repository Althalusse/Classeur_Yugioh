import os
import sqlite3
from tkinter import messagebox
from module.centralisation_dossier import AFFICHER_CARTE
from module.gestion_rarete.tri_carte import sort_cartes
from urllib.parse import urlparse



def get_image_filename_from_url(url):
    if not url:
        return None
    path = urlparse(url).path
    filename = os.path.basename(path)
    return filename


def get_cartes_info(code_set):
    cartes = []
    db_path = os.path.join(AFFICHER_CARTE, code_set, f"{code_set}.db")
    if not os.path.exists(db_path):
        raise FileNotFoundError(f"La base de données du classeur {code_set} n'existe pas.")

    conn = sqlite3.connect(db_path)
    cursor = conn.cursor()
    query = """
        SELECT card_images_image_url, name, card_sets_set_rarity, \
               card_sets_set_code, possessed, card_sets_set_name
        FROM cards
        WHERE card_images_image_url IS NOT NULL
    """
    cursor.execute(query)
    for row in cursor.fetchall():
        url, name, rarity, code, possessed, set_name = row
        img_filename = get_image_filename_from_url(url)
        cartes.append({
            "image_filename": img_filename,
            "name": name,
            "rarity": rarity,
            "code": code,
            "set_code": code,
            "set_rarity": rarity,
            "set_name": set_name,
            "possessed": possessed
        })
    conn.close()
    cartes = sort_cartes(cartes)
    return cartes



# Ce script fournit des fonctions utilitaires pour manipuler les bases de données des classeurs Yu-Gi-Oh!.
# Il permet notamment :
# - De vérifier et ajouter la colonne 'possessed' dans la table 'cards' si elle n'existe pas.
# - De récupérer les informations des cartes d'un classeur (avec image, rareté, code, possession, etc.).
# - De mettre à jour le statut 'possessed' (possédée ou non) d'une carte ou de toutes les cartes d'un classeur.
# - De trier les cartes via la fonction sort_cartes.
# - Il utilise sqlite3 pour accéder aux bases de données et messagebox pour afficher les erreurs à l'utilisateur.
# Ce fichier ne contient pas d'interface graphique complète, mais affiche des messages d'erreur via des popups Tkinter.
