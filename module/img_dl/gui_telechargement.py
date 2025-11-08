import os
import sqlite3
from urllib.parse import urlparse
import threading
from tkinter import messagebox
from module.centralisation_dossier import CLASSEUR_FOLDER, IMG_FOLDER

class TelechargementGUI:
    def __init__(self, service, manager):
        self.service = service
        self.manager = manager

    def telecharger_images_gui(self, code_set, progress_callback=None, on_complete=None):
        try:
            images_folder = os.path.join(IMG_FOLDER, code_set)
            os.makedirs(images_folder, exist_ok=True)

            if progress_callback:
                progress_callback(0, 100, "Initialisation du téléchargement...")

            images_a_telecharger = self._get_images_a_telecharger(code_set, images_folder)

            if not images_a_telecharger:
                self._handle_no_images(progress_callback, on_complete)
                return

            thread = threading.Thread(
                target=self._process_telechargement,
                args=(images_a_telecharger, progress_callback, on_complete),
                daemon=True
            )
            thread.start()

        except Exception as e:
            self._handle_error(e, progress_callback, on_complete)

    def _get_images_a_telecharger(self, code_set, images_folder):
        db_path = os.path.join(CLASSEUR_FOLDER, code_set, f"{code_set}.db")
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
        for (url,) in urls:
            if not url:
                continue
            filename = os.path.basename(urlparse(url).path)
            dest_path = os.path.join(images_folder, filename)
            if not os.path.exists(dest_path):
                images_a_telecharger.append((url, dest_path))
        return images_a_telecharger

    def _process_telechargement(self, images_a_telecharger, progress_callback, on_complete):
        try:
            total = len(images_a_telecharger)
            for i, (image_url, image_path) in enumerate(images_a_telecharger, 1):
                try:
                    self.service.telecharger_image(image_url, image_path)
                    if progress_callback:
                        self._update_progress(progress_callback, i, total)
                except Exception as e:
                    print(f"Erreur lors du téléchargement de {image_url}: {e}")
            
            self._complete_download(on_complete)

        except Exception as e:
            self._handle_error(e, progress_callback, on_complete)

    def _update_progress(self, progress_callback, current, total):
        try:
            progress_callback(current, total, f"Téléchargement de l'image {current}/{total}")
        except Exception:
            pass

    def _handle_no_images(self, progress_callback, on_complete):
        if progress_callback:
            progress_callback(100, 100, "Toutes les images sont déjà téléchargées")
        if on_complete:
            on_complete()

    def _handle_error(self, error, progress_callback, on_complete):
        if progress_callback:
            progress_callback(0, 100, f"❌ Erreur : {str(error)}")
        messagebox.showerror("Erreur", f"Erreur lors du téléchargement : {error}")
        if on_complete:
            on_complete()

    def _complete_download(self, on_complete):
        if on_complete:
            try:
                on_complete()
            except Exception:
                pass

            