import os
import requests
import time
from ratelimit import limits, sleep_and_retry
from module.centralisation_dossier import DEFAULT_IMAGE_PATH

# Constantes
RATE_LIMIT = 20
SECOND = 1

class TelechargementService:
    def __init__(self):
        self.rate_limit = RATE_LIMIT
        self.second = SECOND

    @sleep_and_retry
    @limits(calls=RATE_LIMIT, period=SECOND)
    def telecharger_image(self, url, chemin_fichier, pbar=None, error_callback=None):
        try:
            # Création du dossier parent s'il n'existe pas
            os.makedirs(os.path.dirname(chemin_fichier), exist_ok=True)

            # Vérification si l'image existe déjà
            if os.path.exists(chemin_fichier):
                return  # L'image existe déjà

            response = requests.get(url, stream=True, timeout=10)
            response.raise_for_status()

            # Sauvegarder l'image
            with open(chemin_fichier, "wb") as file:
                for chunk in response.iter_content(1024):
                    if chunk:
                        file.write(chunk)
            if pbar is not None:
                pbar.update(1)
                
        except (requests.exceptions.RequestException, Exception) as e:
            if error_callback:
                error_callback(f"Erreur pour l'URL {url}: {e}")
            else:
                # Utiliser l'image par défaut
                self._utiliser_image_par_defaut(chemin_fichier)

    def _utiliser_image_par_defaut(self, chemin_fichier):
        if not os.path.exists(DEFAULT_IMAGE_PATH):
            self.telecharger_image("URL_DE_L_IMAGE_PAR_DEFAUT", DEFAULT_IMAGE_PATH)
        with open(DEFAULT_IMAGE_PATH, "rb") as src_file:
            with open(chemin_fichier, "wb") as dest_file:
                dest_file.write(src_file.read())