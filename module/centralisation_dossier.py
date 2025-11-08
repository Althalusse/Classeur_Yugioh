import sys
import os


def get_exe_dir():
    """Retourne le dossier où se trouve l'exécutable ou le script principal."""
    if getattr(sys, 'frozen', False):
        # Utilise le dossier du .exe, pas le dossier temporaire
        return os.path.dirname(os.path.abspath(sys.argv[0]))
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


ROOT_FOLDER = get_exe_dir()
BDD_FOLDER = os.path.join(ROOT_FOLDER, "bdd")
IMG_FOLDER = os.path.join(ROOT_FOLDER, "img")
MODULE_FOLDER = os.path.join(ROOT_FOLDER, "module")
EXPORT_FOLDER = os.path.join(ROOT_FOLDER, "export")
DEFAULT_IMAGE_PATH = os.path.join(IMG_FOLDER, "notfound.jpg")
MODULE_IMG_FOLDER = os.path.join(MODULE_FOLDER, "img")



# Chemin vers le dossier de la base de données des classeurs
CLASSEUR_FOLDER = os.path.join(BDD_FOLDER, "classeur_creer")
os.makedirs(CLASSEUR_FOLDER, exist_ok=True)
os.makedirs(IMG_FOLDER, exist_ok=True)
os.makedirs(EXPORT_FOLDER, exist_ok=True)
os.makedirs(BDD_FOLDER, exist_ok=True)

# Chemin vers le dossier Cardmarket
#CARDMARKET_DIR = os.path.join(BDD_FOLDER, "cardmarket")
#os.makedirs(CARDMARKET_DIR, exist_ok=True)

CARDINFO_DB = os.path.join(BDD_FOLDER, "cardinfo.db")
AFFICHER_CARTE = CLASSEUR_FOLDER
CONFIG_FILE = os.path.join(BDD_FOLDER, "rarity_config.json")

# Chemins centralisés pour la gestion de la base de données et des sauvegardes
DB_PATH = CARDINFO_DB
BACKUP_DIR = os.path.join(ROOT_FOLDER, "backups")
LAST_UPDATE_FILE = os.path.join(BDD_FOLDER, "last_update.txt")
FIRST_RUN_FILE = os.path.join(ROOT_FOLDER, "first_run.flag")
os.makedirs(BACKUP_DIR, exist_ok=True)

PATHS = {
    "root": ROOT_FOLDER,
    "bdd": BDD_FOLDER,
    "export": EXPORT_FOLDER,
    "img": IMG_FOLDER,
    "classeur": CLASSEUR_FOLDER,
#    "cardmarket": CARDMARKET_DIR
}


def get_paths():
    """Renvoie tous les chemins essentiels du projet sous forme de dictionnaire."""
    return PATHS
