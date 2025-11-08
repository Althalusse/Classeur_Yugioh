import os
import sqlite3
from module.centralisation_dossier import CLASSEUR_FOLDER

# Ce fichier contient la logique métier/fonctionnelle :
# - Accès et manipulation de la base de données (récupération, suppression, ajout, etc.)
# - Traitement des données (formatage, vérification de colonnes, etc.)
# - Aucune dépendance à Tkinter ou à l'UI

def get_cartes_possedees():
    """
    Parcourt tous les classeurs et récupère les cartes possédées (possessed=1).
    Retourne une liste de dictionnaires avec :
    - name
    - card_sets_set_name
    - card_sets_set_code
    - quantite
    - card_sets_set_rarity
    - classeur
    """
    result = []
    if not os.path.exists(CLASSEUR_FOLDER):
        return result
    for classeur in os.listdir(CLASSEUR_FOLDER):
        classeur_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
        if not os.path.exists(classeur_path):
            continue
        try:
            conn = sqlite3.connect(classeur_path)
            cursor = conn.cursor()
            
            # Vérifier si la colonne qualite existe
            cursor.execute("PRAGMA table_info(cards)")
            columns = [col[1] for col in cursor.fetchall()]
            if "qualite" not in columns:
                cursor.execute("ALTER TABLE cards ADD COLUMN qualite TEXT DEFAULT NULL")
                conn.commit()

            # Récupérer les cartes avec la qualité
            cursor.execute("""
                SELECT name, card_sets_set_name, card_sets_set_code, 
                       quantite, card_sets_set_rarity, qualite
                FROM cards 
                WHERE possessed = 1
            """)
            
            # Permet la sélection multiple par glisser (drag) dans l'UI (Treeview)
            # (Aucune modification ici, mais à appliquer côté UI :
            #   tree = ttk.Treeview(..., selectmode='extended')
            #   tree.bind('<B1-Motion>', lambda e: tree.selection_set(tree.identify_row(e.y)))
            # )
            for name, set_name, set_code, quantite, rarity , qualite in cursor.fetchall():
                # Si possessed=1, quantité doit être au moins 1
                quantite = max(1, quantite if quantite is not None else 1)
                result.append({
                    "name": name,
                    "card_sets_set_name": set_name,
                    "card_sets_set_code": set_code,
                    "quantite": quantite,
                    "classeur": classeur,
                    "card_sets_set_rarity": rarity,
                    "qualite": qualite or ""  # Si NULL, retourner une chaîne vide
                })
            conn.close()
        except Exception as e:
            print(f"[ERREUR] {classeur}: {e}")
    return result

def supprimer_carte_possedee(classeur, nom_carte, set_code):
    """
    Supprime une carte de l'inventaire en mettant possessed à 0 et quantité à 0.
    """
    try:
        db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        cursor.execute("""
            UPDATE cards 
            SET possessed = 0, quantite = 0 
            WHERE name = ? AND card_sets_set_code = ?
        """, (nom_carte, set_code))
        conn.commit()
        conn.close()
        return True
    except Exception as e:
        print(f"[ERREUR] Suppression carte {nom_carte} du classeur {classeur}: {e}")
        return False

if __name__ == "__main__":
    # Affichage console pour test
    for carte in get_cartes_possedees():
        print(f"{carte['name']} ; {carte['card_sets_set_name']} ; {carte['card_sets_set_code']} ; {carte['quantite']}")
