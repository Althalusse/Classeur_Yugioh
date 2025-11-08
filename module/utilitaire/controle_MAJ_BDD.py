import os
import shutil
import requests
import json
from datetime import datetime
try:
    from module import BDD_creation
    from module.centralisation_dossier import get_paths
except ImportError:
    import BDD_creation
    from module.centralisation_dossier import get_paths

# Chemins des fichiers
PATHS = get_paths()
DB_PATH = os.path.join(PATHS["bdd"], "yugioh_cards.db")
LAST_UPDATE_FILE = os.path.join(PATHS["bdd"], "last_update.txt")
BACKUP_DIR = os.path.join(PATHS["bdd"], "backups")

# Créer le dossier de sauvegarde s'il n'existe pas
os.makedirs(BACKUP_DIR, exist_ok=True)

# Le chemin LAST_UPDATE_FILE est déjà défini plus haut

def check_db_version():
    """
    Vérifie la version de la base de données sur l'API YGOPRODECK
    
    Returns:
        list: Liste des informations de version de la base de données
        None: En cas d'erreur
    """
    url = "https://db.ygoprodeck.com/api/v7/checkDBVer.php"
    try:
        response = requests.get(url, timeout=10)
        response.raise_for_status()
        return response.json()
    except requests.exceptions.RequestException as e:
        print(f"Erreur réseau lors de la vérification de la version : {e}")
        return None
    except json.JSONDecodeError as e:
        print(f"Erreur lors du décodage de la réponse : {e}")
        return None

def get_version_info():
    """
    Récupère les informations de version actuelles (locale et distante)
    
    Returns:
        dict: Dictionnaire contenant les informations de version avec les clés :
            - 'remote' (str): Version distante
            - 'local' (str): Version locale
            - 'update_available' (bool): Si une mise à jour est disponible
            - 'error' (str, optionnel): Message d'erreur en cas de problème
    """
    try:
        # Récupérer la version distante
        db_version_info = check_db_version()
        if not db_version_info:
            return {
                'error': 'Impossible de récupérer la version distante',
                'remote': 'Inconnue',
                'local': 'Inconnue',
                'update_available': False
            }
            
        remote_version = db_version_info[0]["last_update"]
        
        # Récupérer la version locale
        try:
            with open(LAST_UPDATE_FILE, "r") as file:
                local_version = file.read().strip()
        except FileNotFoundError:
            local_version = "Non installée"
            
        return {
            'remote': remote_version,
            'local': local_version,
            'update_available': local_version != remote_version and local_version != "Non installée"
        }
        
    except Exception as e:
        return {
            'error': str(e),
            'remote': 'Erreur',
            'local': 'Erreur',
            'update_available': False
        }

def check_for_updates():
    """
    Vérifie si une mise à jour de la base de données est disponible.
    
    Returns:
        tuple: (bool, dict) - 
            - bool: True si une mise à jour est disponible
            - dict: Dictionnaire contenant les informations de version
    """
    version_info = get_version_info()
    if 'error' in version_info:
        return False, version_info
    return version_info['update_available'], version_info



