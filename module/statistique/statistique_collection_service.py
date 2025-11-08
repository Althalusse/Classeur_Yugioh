import os
import sqlite3
from module.centralisation_dossier import CLASSEUR_FOLDER

def get_stats_collection():
    """
    Récupère les statistiques de toutes les collections
    Retourne une liste de dictionnaires contenant les informations de chaque collection
    """
    stats = []
    for nom_classeur in os.listdir(CLASSEUR_FOLDER):
        classeur_path = os.path.join(CLASSEUR_FOLDER, nom_classeur)
        db_file = os.path.join(classeur_path, f"{nom_classeur}.db")
        if not os.path.isfile(db_file):
            continue
        try:
            conn = sqlite3.connect(db_file)
            cursor = conn.cursor()
            cursor.execute("SELECT COUNT(*) FROM cards")
            total = cursor.fetchone()[0]
            cursor.execute("SELECT COUNT(*) FROM cards WHERE possessed = 1")
            possedees = cursor.fetchone()[0]
            cursor.execute("""
                SELECT card_sets_set_rarity, COUNT(*) as total, 
                       SUM(CASE WHEN possessed = 1 THEN 1 ELSE 0 END) as possedees
                FROM cards
                WHERE card_sets_set_rarity IS NOT NULL
                GROUP BY card_sets_set_rarity
            """)
            raretes = {}
            for rarity, total_rarete, possedees_rarete in cursor.fetchall():
                if rarity:
                    raretes[rarity] = {
                        'total': total_rarete,
                        'possedees': possedees_rarete,
                        'pourcentage': (possedees_rarete / total_rarete * 100) if total_rarete > 0 else 0
                    }
            stats.append({
                'nom': nom_classeur,
                'total': total,
                'possedees': possedees,
                'pourcentage': (possedees / total * 100) if total > 0 else 0,
                'raretes': raretes
            })
            conn.close()
        except Exception as e:
            print(f"Erreur avec {nom_classeur}: {e}")
    return sorted(stats, key=lambda x: x['nom'])

def stats_par_collection():
    stats = get_stats_collection()
    print(f"{'Collection':<12} | {'Possédées':<10} | {'Total':<6} | {'% Complétion':<12}")
    print("-" * 50)
    for stat in stats:
        print(f"{stat['nom']:<12} | {stat['possedees']:<10} | {stat['total']:<6} | {stat['pourcentage']:10.2f} %")

if __name__ == "__main__":
    stats_par_collection()
