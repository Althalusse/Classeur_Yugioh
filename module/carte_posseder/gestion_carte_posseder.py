import os
import sqlite3
from tkinter import messagebox
from module.centralisation_dossier import AFFICHER_CARTE

def update_possessed_in_db(db_path, set_code, set_rarity, possessed_value):
    """
    Met à jour le statut 'possessed' d'une carte spécifique dans la base de données.
    """
    try:
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        cursor.execute(
            "UPDATE cards SET possessed = ? WHERE card_sets_set_code = ? AND card_sets_set_rarity = ?",
            (possessed_value, set_code, set_rarity)
        )
        conn.commit()
        conn.close()
    except Exception as e:
        messagebox.showerror("Erreur BDD", f"Impossible de mettre à jour la carte:\n{e}")

def set_all_possessed(code_set, valeur):
    """
    Met à jour le statut 'possessed' de toutes les cartes d'un classeur.
    """
    db_path = os.path.join(AFFICHER_CARTE, code_set, f"{code_set}.db")
    try:
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        cursor.execute("UPDATE cards SET possessed = ?", (valeur,))
        conn.commit()
        conn.close()
    except Exception as e:
        messagebox.showerror("Erreur BDD", f"Impossible de mettre à jour toutes les cartes:\n{e}")