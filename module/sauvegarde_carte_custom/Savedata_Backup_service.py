import os
import sqlite3
import json
import shutil
from datetime import datetime
from module.centralisation_dossier import ROOT_FOLDER, CLASSEUR_FOLDER

BACKUP_FOLDER = os.path.join(ROOT_FOLDER, "backups")
os.makedirs(BACKUP_FOLDER, exist_ok=True)

def create_backup():
    """Crée une sauvegarde complète des cartes personnalisées et des configurations"""
    try:
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        backup_data = {
            "custom_cards": {},
            "classeurs": [],
            "timestamp": timestamp
        }

        # Parcourir tous les classeurs
        for classeur in os.listdir(CLASSEUR_FOLDER):
            classeur_path = os.path.join(CLASSEUR_FOLDER, classeur)
            if not os.path.isdir(classeur_path):
                continue

            db_path = os.path.join(classeur_path, f"{classeur}.db")
            if not os.path.exists(db_path):
                continue

            conn = sqlite3.connect(db_path)
            cursor = conn.cursor()

            # Vérifier la colonne is_custom
            cursor.execute("PRAGMA table_info(cards)")
            columns = [col[1] for col in cursor.fetchall()]
            if "is_custom" not in columns:
                conn.close()
                continue

            # Récupérer toutes les cartes personnalisées
            cursor.execute("""
                SELECT name, card_sets_set_code, card_sets_set_rarity, 
                       description, card_images_image_url, possessed
                FROM cards
                WHERE is_custom = 1
            """)
            
            custom_cards = cursor.fetchall()
            if custom_cards:
                backup_data["classeurs"].append(classeur)  # Ajoute le classeur seulement s'il y a des cartes custom
                backup_data["custom_cards"][classeur] = [{
                    "classeur": classeur,
                    "name": card[0],
                    "code": card[1],
                    "rarity": card[2],
                    "description": card[3],
                    "image_url": card[4],
                    "possessed": card[5]
                } for card in custom_cards]
            
            conn.close()

        # Sauvegarder dans un fichier JSON
        backup_file = os.path.join(BACKUP_FOLDER, f"backup_{timestamp}.json")
        with open(backup_file, "w", encoding="utf-8") as f:
            json.dump(backup_data, f, indent=4, ensure_ascii=False)

        # Nettoyer les anciennes sauvegardes (garder les 5 plus récentes)
        cleanup_old_backups()

        return backup_file

    except Exception as e:
        raise Exception(f"Erreur lors de la sauvegarde : {str(e)}")

def restore_backup(backup_file=None):
    """Restaure une sauvegarde"""
    try:
        if backup_file is None:
            # Utiliser la sauvegarde la plus récente
            backups = get_available_backups()
            if not backups:
                raise Exception("Aucune sauvegarde trouvée")
            backup_file = backups[-1]["path"]

        with open(backup_file, "r", encoding="utf-8") as f:
            backup_data = json.load(f)

        for classeur, cards in backup_data["custom_cards"].items():
            db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
            if not os.path.exists(os.path.dirname(db_path)):
                os.makedirs(os.path.dirname(db_path))

            conn = sqlite3.connect(db_path)
            cursor = conn.cursor()

            # Vérifier/créer la colonne is_custom
            cursor.execute("PRAGMA table_info(cards)")
            columns = [col[1] for col in cursor.fetchall()]
            if "is_custom" not in columns:
                cursor.execute("ALTER TABLE cards ADD COLUMN is_custom INTEGER DEFAULT 0")

            # Supprimer les anciennes cartes personnalisées
            cursor.execute("DELETE FROM cards WHERE is_custom = 1")

            # Restaurer les cartes
            for card in cards:
                cursor.execute("""
                    INSERT INTO cards (
                        name, card_sets_set_code, card_sets_set_rarity,
                        description, card_images_image_url, is_custom, possessed
                    )
                    VALUES (?, ?, ?, ?, ?, 1, ?)
                """, (
                    card["name"],
                    card["code"],
                    card["rarity"],
                    card["description"],
                    card["image_url"],
                    card["possessed"]
                ))

            conn.commit()
            conn.close()

    except Exception as e:
        raise Exception(f"Erreur lors de la restauration : {str(e)}")

def get_available_backups():
    """Retourne la liste des sauvegardes disponibles"""
    backups = []
    for file in os.listdir(BACKUP_FOLDER):
        if file.startswith("backup_") and file.endswith(".json"):
            path = os.path.join(BACKUP_FOLDER, file)
            timestamp = file[7:-5]  # Extrait la date/heure du nom du fichier
            backups.append({
                "path": path,
                "timestamp": timestamp,
                "date": datetime.strptime(timestamp, "%Y%m%d_%H%M%S")
            })
    return sorted(backups, key=lambda x: x["date"])

def cleanup_old_backups(keep=5):
    """Nettoie les anciennes sauvegardes en gardant uniquement les plus récentes"""
    backups = get_available_backups()
    for backup in backups[:-keep]:  # Garde les 5 plus récentes
        try:
            os.remove(backup["path"])
        except Exception:
            pass

def delete_backup(backup_file):
    """Supprime une sauvegarde spécifique"""
    try:
        if os.path.exists(backup_file):
            os.remove(backup_file)
        else:
            raise Exception("Le fichier de sauvegarde n'existe pas.")
    except Exception as e:
        raise Exception(f"Erreur lors de la suppression de la sauvegarde : {str(e)}")