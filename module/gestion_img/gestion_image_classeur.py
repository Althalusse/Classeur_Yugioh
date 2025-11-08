import os
import tkinter as tk
from tkinter import messagebox
from PIL import Image, ImageTk, UnidentifiedImageError
from module.centralisation_dossier import IMG_FOLDER, DEFAULT_IMAGE_PATH
from tkinter import filedialog
import shutil
import time


def get_image_path(classeur, image_filename):
    """Retourne le chemin absolu de l'image pour un classeur donné."""
    return os.path.join(IMG_FOLDER, classeur, image_filename)

def load_image(path, size=(190, 230)):
    """Charge et redimensionne une image. Vérifie toujours l'existence de l'image originale."""
    try:
        # Toujours vérifier si l'image d'origine existe
        if os.path.exists(path):
            try:
                pil_img = Image.open(path)
                # Vérifier que l'image est valide
                pil_img.verify()
                # Réouvrir l'image après verify() car verify() la ferme
                pil_img = Image.open(path)
                pil_img_resized = pil_img.resize(size)
                return ImageTk.PhotoImage(pil_img_resized)
            except Exception:
                # Si l'image est corrompue, utiliser l'image par défaut
                if os.path.exists(DEFAULT_IMAGE_PATH):
                    pil_img = Image.open(DEFAULT_IMAGE_PATH)
                    pil_img_resized = pil_img.resize(size)
                    return ImageTk.PhotoImage(pil_img_resized)
        else:
            # Si l'image n'existe pas, utiliser l'image par défaut
            if os.path.exists(DEFAULT_IMAGE_PATH):
                pil_img = Image.open(DEFAULT_IMAGE_PATH)
                pil_img_resized = pil_img.resize(size)
                return ImageTk.PhotoImage(pil_img_resized)
    except Exception:
        pass
    
    # En dernier recours, retourner une image grise
    return get_placeholder_image(size)

def get_placeholder_image(size=(190, 230)):
    """Retourne une image grise de remplacement."""
    pil_img = Image.new('RGB', size, color='grey')
    return ImageTk.PhotoImage(pil_img)

def charger_image_absente(chemin_image):
    try:
        return Image.open(chemin_image)
    except (FileNotFoundError, UnidentifiedImageError, OSError):
        return Image.open(DEFAULT_IMAGE_PATH)

def copier_image_personnalisee(nom_carte):
    """
    Ouvre une boîte de dialogue pour sélectionner une image, copie l'image dans le dossier custom_images,
    la renomme de façon unique (nom de la carte + timestamp), et retourne le chemin relatif à utiliser.
    Retourne None si l'utilisateur annule.
    """
    file_path = filedialog.askopenfilename(
        title="Sélectionner une image",
        filetypes=[("Images", "*.png *.jpg *.jpeg *.gif")]
    )
    if not file_path:
        return None
    dossier_base = os.path.dirname(os.path.dirname(__file__))
    dossier_custom = os.path.join(dossier_base, "custom_images")
    os.makedirs(dossier_custom, exist_ok=True)
    ext = os.path.splitext(file_path)[1]
    nom_fichier = f"{nom_carte}_{int(time.time())}{ext}"
    chemin_cible = os.path.join(dossier_custom, nom_fichier)
    shutil.copy2(file_path, chemin_cible)
    chemin_relatif = os.path.relpath(chemin_cible, dossier_base)
    return chemin_relatif
