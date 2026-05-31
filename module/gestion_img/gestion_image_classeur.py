"""
gestion_image_classeur.py — Chargement et gestion des images de cartes.

Stratégie de recherche d'image (par ordre de priorité) :
  1. img/small/{filename}   — pool partagé (classeurs créés via YGOPRODeck API)
  2. img/{SET_CODE}/{filename} — pool par classeur (anciens classeurs YGOJSON)
  3. Image de remplacement (notfound.jpg) ou placeholder gris
"""

# Yu-Gi-Oh! Collection Manager
# Copyright (C) 2026  Althalusse
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

import os
import tkinter as tk
from tkinter import messagebox
from PIL import Image, ImageTk, UnidentifiedImageError
from tkinter import filedialog
import shutil
import time

from module.centralisation_dossier import (
    IMG_FOLDER, IMAGES_SMALL_FOLDER, DEFAULT_IMAGE_PATH, ROOT_FOLDER
)


# ─────────────────────────────────────────────────────────────────────────────
# Chemin d'image
# ─────────────────────────────────────────────────────────────────────────────

def get_image_path(classeur: str, image_filename: str) -> str:
    """
    Retourne le chemin absolu de l'image pour un classeur donné.

    Cherche d'abord dans le pool partagé img/small/ (nouveau système API),
    puis dans img/{classeur}/ (compatibilité ascendante classeurs YGOJSON).
    """
    if not image_filename:
        return DEFAULT_IMAGE_PATH

    # Pool partagé (YGOPRODeck API — spec §5)
    shared = os.path.join(IMAGES_SMALL_FOLDER, image_filename)
    if os.path.exists(shared):
        return shared

    # Pool par classeur (ancien système YGOJSON)
    per_classeur = os.path.join(IMG_FOLDER, classeur, image_filename)
    if os.path.exists(per_classeur):
        return per_classeur

    # Aucun fichier trouvé — retourne le chemin partagé (sera affiché en placeholder)
    return shared



# ─────────────────────────────────────────────────────────────────────────────
# Chargement d'image
# ─────────────────────────────────────────────────────────────────────────────


def get_placeholder_image(size=(190, 230)):
    """Retourne une image grise de remplacement."""
    return ImageTk.PhotoImage(Image.new("RGB", size, color="grey"))




# ─────────────────────────────────────────────────────────────────────────────
# Détection du placeholder « notfound »
# ─────────────────────────────────────────────────────────────────────────────

# Cache module-level : évite de recalculer (taille, hash) de DEFAULT_IMAGE_PATH
# à chaque appel. Invalidé automatiquement si le fichier change de taille ou
# de mtime (détection en 2 os.stat, quasi gratuit).
_notfound_signature: tuple | None = None  # (size, mtime, md5_digest)


def _signature_notfound() -> tuple | None:
    """
    Retourne (size, mtime, md5_digest) de DEFAULT_IMAGE_PATH, ou None si absent.
    Mise en cache invalidée si (size, mtime) changent.
    """
    global _notfound_signature
    if not os.path.exists(DEFAULT_IMAGE_PATH):
        _notfound_signature = None
        return None
    try:
        st = os.stat(DEFAULT_IMAGE_PATH)
    except OSError:
        _notfound_signature = None
        return None
    cur_stat = (st.st_size, st.st_mtime)
    if (_notfound_signature is not None
            and _notfound_signature[0] == cur_stat[0]
            and _notfound_signature[1] == cur_stat[1]):
        return _notfound_signature
    # (Re)calcul du hash
    try:
        import hashlib
        h = hashlib.md5()
        with open(DEFAULT_IMAGE_PATH, "rb") as f:
            for chunk in iter(lambda: f.read(8192), b""):
                h.update(chunk)
        _notfound_signature = (st.st_size, st.st_mtime, h.digest())
        return _notfound_signature
    except Exception:
        _notfound_signature = None
        return None


def est_notfound_placeholder(chemin_fichier: str) -> bool:
    """
    Détermine si un fichier image est un placeholder notfound.jpg posé par
    le worker de téléchargement (cf. TelechargementService._utiliser_image_par_defaut).

    Critères (stricts) :
      1. Le fichier existe.
      2. DEFAULT_IMAGE_PATH existe également (sinon, on ne peut pas comparer).
      3. Taille identique à DEFAULT_IMAGE_PATH.
      4. Hash MD5 identique à DEFAULT_IMAGE_PATH.

    Utilisé à l'ouverture d'un classeur pour identifier les cartes dont
    l'image précédente avait échoué et qui doivent être re-tentées.
    """
    if not chemin_fichier or not os.path.exists(chemin_fichier):
        return False
    sig = _signature_notfound()
    if sig is None:
        return False
    ref_size, _, ref_md5 = sig
    try:
        if os.path.getsize(chemin_fichier) != ref_size:
            return False
    except OSError:
        return False
    try:
        import hashlib
        h = hashlib.md5()
        with open(chemin_fichier, "rb") as f:
            for chunk in iter(lambda: f.read(8192), b""):
                h.update(chunk)
        return h.digest() == ref_md5
    except Exception:
        return False
