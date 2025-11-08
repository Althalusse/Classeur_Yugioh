"""
Module pour contrôler la version de la base de données distante (API YGOProDeck) et gérer le fichier last_update.txt.
"""
import requests
import os
import json
import shutil
from datetime import datetime

# Chemins et constantes centralisés
from module.centralisation_dossier import DB_PATH, BACKUP_DIR, LAST_UPDATE_FILE

# Corriger l'import de BDD_creation
from module import BDD_creation

def check_db_version():
    """
    Récupère les informations de version de la base de données distante via l'API YGOProDeck.
    Retourne le JSON de version (list ou dict).
    """
    url = "https://db.ygoprodeck.com/api/v7/checkDBVer.php"
    response = requests.get(url, timeout=10)
    response.raise_for_status()
    return response.json()

def check_for_updates():
    """
    Vérifie si une mise à jour de la base de données est disponible.
    Retourne (True/False, infos de version ou erreur).
    """
    try:
        remote_info = check_db_version()
        local_info = load_last_update_file(LAST_UPDATE_FILE)
        
        # Correction de l'accès aux versions
        remote_version = remote_info[0]["database_version"] if isinstance(remote_info, list) else None
        local_version = local_info[0]["database_version"] if local_info and isinstance(local_info, list) else None
        
        if remote_version is None:
            return False, {"error": "Version distante non disponible"}
            
        update_available = (local_version != remote_version)
        return update_available, {"local": local_version or "Aucune", "remote": remote_version}
    except Exception as e:
        return False, {"error": str(e)}

def save_last_update_file(json_data, last_update_file_path):
    """
    Sauvegarde le JSON complet de version dans le fichier last_update.txt.
    S'assure que les données sont au bon format avant la sauvegarde.
    """
    # Normalisation des données pour s'assurer qu'elles sont au bon format
    if isinstance(json_data, list):
        data_to_save = json_data
    else:
        data_to_save = [json_data]  # Encapsuler dans une liste si ce n'est pas déjà une liste
        
    with open(last_update_file_path, "w", encoding="utf-8") as f:
        json.dump(data_to_save, f, indent=2, ensure_ascii=False)

def load_last_update_file(last_update_file_path):
    """
    Charge le JSON du fichier last_update.txt avec gestion des erreurs.
    """
    if not os.path.exists(last_update_file_path):
        return None
        
    try:
        with open(last_update_file_path, "r", encoding="utf-8") as f:
            data = json.load(f)
            # S'assurer que les données sont au format attendu
            if isinstance(data, list):
                return data
            return [data]  # Encapsuler dans une liste si ce n'est pas déjà une liste
    except json.JSONDecodeError:
        # Si le fichier est corrompu, le supprimer et retourner None
        os.remove(last_update_file_path)
        return None
    except Exception:
        return None

def create_backup():
    """Crée une sauvegarde de la base de données actuelle"""
    try:
        if not os.path.exists(DB_PATH):
            return True  # Pas de sauvegarde nécessaire si la BDD n'existe pas
            
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        backup_path = os.path.join(BACKUP_DIR, f"yugioh_cards_backup_{timestamp}.db")
        shutil.copy2(DB_PATH, backup_path)
        return True
    except Exception as e:
        # Ne pas faire de print ni de popup ici, juste retourner False
        return False

def update_database():
    """
    Met à jour la base de données avec les dernières données.
    Returns:
        tuple: (bool, str) - (True si réussi, message d'information)
    """
    try:
        # Créer une sauvegarde avant la mise à jour
        if not create_backup():
            return False, "Échec de la création de la sauvegarde"

        # Télécharger les nouvelles données
        card_data = BDD_creation.fetch_card_data()  # Plus besoin de spécifier la langue
        df = BDD_creation.transform_data(card_data["data"])

        # Recréer la base de données avec les nouvelles données
        BDD_creation.create_dynamic_table(df)
        BDD_creation.insert_data_to_db(df)

        # Mettre à jour le fichier de version avec le JSON complet
        remote_info = check_db_version()
        save_last_update_file(remote_info, LAST_UPDATE_FILE)

        return True, "Base de données mise à jour avec succès."

    except Exception as e:
        return False, f"Erreur lors de la mise à jour de la base de données : {str(e)}"