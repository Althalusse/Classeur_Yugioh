import os
import sqlite3
from urllib.parse import urlparse
from concurrent.futures import ThreadPoolExecutor
from tqdm import tqdm
import threading
import time
from module.centralisation_dossier import CLASSEUR_FOLDER, IMG_FOLDER

class TelechargementManager:
    def __init__(self, service):
        self.service = service

    def telecharger_images(self, code_set):
        try:
            db_path = os.path.join(CLASSEUR_FOLDER, code_set, f"{code_set}.db")
            if not os.path.exists(db_path):
                raise FileNotFoundError(f"Base de données introuvable: {db_path}")

            images_folder = os.path.join(IMG_FOLDER, code_set)
            os.makedirs(images_folder, exist_ok=True)

            images_a_telecharger = self._get_images_a_telecharger(db_path, images_folder)
            if not images_a_telecharger:
                return

            with tqdm(
                total=len(images_a_telecharger),
                desc=f"Téléchargement des images pour {code_set}",
                unit="image",
            ) as pbar:
                with ThreadPoolExecutor(max_workers=5) as executor:
                    futures = [
                        executor.submit(self.service.telecharger_image, url, path, pbar)
                        for url, path in images_a_telecharger
                    ]
                    for future in futures:
                        future.result()

        except Exception as e:
            raise Exception(
                f"Erreur lors du téléchargement des images pour {code_set}: {e}"
            )

    def _get_images_a_telecharger(self, db_path, images_folder):
        conn = sqlite3.connect(db_path)
        cursor = conn.cursor()
        cursor.execute("""
            SELECT card_images_image_url 
            FROM cards 
            WHERE card_images_image_url IS NOT NULL AND card_images_image_url != ''
        """)
        urls = cursor.fetchall()
        conn.close()

        images_a_telecharger = []
        for url in urls:
            if not url[0]:
                continue
            filename = os.path.basename(urlparse(url[0]).path)
            dest_path = os.path.join(images_folder, filename)
            if not os.path.exists(dest_path):
                images_a_telecharger.append((url[0], dest_path))

        return images_a_telecharger