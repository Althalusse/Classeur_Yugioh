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
import requests
import time
from PIL import Image
from ratelimit import limits, sleep_and_retry
from module.centralisation_dossier import DEFAULT_IMAGE_PATH

# Bytes PNG 1×1 transparent — fallback ultime si PIL lui-même échoue
# (cas très improbable : environnement Pillow cassé). Garantit au minimum
# l'existence d'un fichier image décodable sur disque.
_PNG_1X1_TRANSPARENT = (
    b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01"
    b"\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xfc\xcf"
    b"\xc0\xc0\xc0\x00\x00\x00\x05\x00\x01\x87\xa6\x9a)\x00\x00\x00\x00IEND"
    b"\xaeB`\x82"
)

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
        """
        Garantit qu'un fichier image valide existe à `chemin_fichier` après l'appel.

        Stratégie à 3 niveaux (du plus propre au plus dégradé) :
          1. Copie de DEFAULT_IMAGE_PATH (notfound.jpg) si présent.
          2. Génération d'un placeholder PIL 80×115 si (1) absent ou échoue.
          3. Écriture de bytes PNG 1×1 bruts si même PIL échoue.

        Important : dans tous les cas, `os.path.exists(chemin_fichier)` doit être
        True en sortie — c'est le contrat sur lequel le worker de la file
        d'attente se base pour ne pas re-queuer infiniment la même carte.
        """
        # Niveau 1 : copie du fichier de référence
        if os.path.exists(DEFAULT_IMAGE_PATH):
            try:
                with open(DEFAULT_IMAGE_PATH, "rb") as src_file:
                    with open(chemin_fichier, "wb") as dest_file:
                        dest_file.write(src_file.read())
                return
            except Exception:
                pass  # bascule sur le niveau 2

        # Niveau 2 : placeholder généré à la volée
        # Fond sombre (assorti au thème), taille standard d'une miniature carte.
        try:
            placeholder = Image.new("RGB", (80, 115), color=(26, 26, 46))
            placeholder.save(chemin_fichier)
            return
        except Exception:
            pass  # bascule sur le niveau 3

        # Niveau 3 : bytes PNG bruts — dernier recours pour garantir l'existence
        try:
            with open(chemin_fichier, "wb") as dest_file:
                dest_file.write(_PNG_1X1_TRANSPARENT)
        except Exception:
            # Si même ça échoue, l'écriture disque est impossible : on laisse
            # l'appelant gérer. On a au moins essayé trois niveaux de fallback.
            pass

    # ─────────────────────────────────────────────────────────────────────────
    # Téléchargement avec fallback multi-sources
    # ─────────────────────────────────────────────────────────────────────────

    @sleep_and_retry
    @limits(calls=RATE_LIMIT, period=SECOND)
    def _tenter_une_url(self, url: str, chemin_fichier: str) -> bool:
        """
        Tente de télécharger `url` vers `chemin_fichier`.
        Retourne True si succès (fichier écrit, > 200 bytes), False sinon.
        N'écrit RIEN en cas d'échec (le fichier cible reste intact).

        Le sanity check 200 bytes évite d'écraser un placeholder existant par
        une page d'erreur HTML renvoyée en 200 OK (cas Yugipedia 404 déguisé).
        """
        if not url:
            return False
        try:
            os.makedirs(os.path.dirname(chemin_fichier), exist_ok=True)
            response = requests.get(url, stream=True, timeout=10)
            response.raise_for_status()

            # Écriture dans un fichier temporaire pour ne pas corrompre le
            # fichier cible si le download échoue en cours de route.
            tmp_path = chemin_fichier + ".part"
            try:
                with open(tmp_path, "wb") as f:
                    for chunk in response.iter_content(1024):
                        if chunk:
                            f.write(chunk)
                # Sanity check : un vrai JPG/PNG de carte fait au moins
                # quelques dizaines de Ko. Une taille < 200 bytes signale
                # presque toujours une page d'erreur.
                if os.path.getsize(tmp_path) < 200:
                    os.remove(tmp_path)
                    return False
                # Atomic move (écrase l'existant si présent)
                os.replace(tmp_path, chemin_fichier)
                return True
            except Exception:
                if os.path.exists(tmp_path):
                    try:
                        os.remove(tmp_path)
                    except Exception:
                        pass
                return False
        except Exception:
            return False

    def telecharger_avec_fallback(self, url_primary: str | None,
                                   url_fallback: str | None,
                                   chemin_fichier: str) -> bool:
        """
        Tente de télécharger une image avec fallback automatique.

        Ordre d'essai :
          1. url_primary  (source active de l'utilisateur)
          2. url_fallback (source alternative)
          3. _utiliser_image_par_defaut (notfound.jpg / placeholder)

        Retourne True si une des deux sources a réussi, False si fallback
        sur l'image par défaut.

        Le fichier cible est toujours présent sur disque après l'appel
        (garanti par _utiliser_image_par_defaut en dernier recours).
        """
        # Essai 1 : source primaire
        if url_primary and self._tenter_une_url(url_primary, chemin_fichier):
            return True

        # Essai 2 : source de fallback
        if url_fallback and self._tenter_une_url(url_fallback, chemin_fichier):
            return True

        # Essai 3 : placeholder (garantit l'existence du fichier)
        self._utiliser_image_par_defaut(chemin_fichier)
        return False
