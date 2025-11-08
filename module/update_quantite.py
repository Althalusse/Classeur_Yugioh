import os
import sqlite3
from module.centralisation_dossier import CLASSEUR_FOLDER

def update_quantite_in_classeur(classeur, name, set_code, quantite, rarete=None):
    """
    Met à jour la quantité d'une carte dans le classeur donné.
    Si une rareté est spécifiée, ne met à jour que la carte avec cette rareté.
    """
    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.exists(db_path):
        return False
        
    try:
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        
        if rarete:
            # Mise à jour avec la rareté spécifiée
            cursor.execute(
                """
                UPDATE cards 
                SET quantite = ? 
                WHERE name = ? 
                AND card_sets_set_code = ? 
                AND TRIM(card_sets_set_rarity) = TRIM(?)
                AND possessed = 1
                """,
                (quantite, name, set_code, rarete.strip())
            )
        else:
            # Ancien comportement pour rétrocompatibilité
            cursor.execute(
                """
                UPDATE cards 
                SET quantite = ? 
                WHERE name = ? 
                AND card_sets_set_code = ? 
                AND possessed = 1
                """,
                (quantite, name, set_code)
            )
            
        conn.commit()
        conn.close()
        return True
    except Exception:
        return False

def update_qualite_in_classeur(classeur, nom_carte, set_code, qualite, rarete):
    """Met à jour la qualité d'une carte dans le classeur."""
    try:
        db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()

        cursor.execute("""
            UPDATE cards 
            SET qualite = ? 
            WHERE name = ? 
            AND card_sets_set_code = ? 
            AND card_sets_set_rarity = ?
            AND possessed = 1
        """, (qualite, nom_carte, set_code, rarete))
        
        conn.commit()
        conn.close()
        return True
    except Exception as e:
        print(f"Erreur lors de la mise à jour de la qualité : {e}")
        return False
