"""
redemarrage.py — Redémarrage de l'application.

Relance une nouvelle instance puis ferme l'instance courante. Utilisé par
l'écran Options quand un réglage ne s'applique qu'au prochain démarrage
(langue de l'interface, taille de police).

Compatible PyInstaller --onefile : relance le .exe lui-même.
  - frozen : [<exe>]
  - dev    : [<python>, <racine>/main.py]

IMPORTANT (bug onefile) : un exe PyInstaller --onefile s'extrait dans un
dossier temporaire `_MEIxxxxx` et le signale via des variables d'environnement
(`_MEIPASS2`, `_PYI*`). Si on relance l'exe SANS nettoyer ces variables,
l'enfant hérite du dossier `_MEIxxxxx` du parent au lieu d'extraire le sien ;
quand le parent se ferme, il SUPPRIME ce dossier et l'enfant perd ses fichiers
(Tcl/Tk introuvable, images cassées, crash « Tcl data directory not found »).
On purge donc ces variables pour forcer l'enfant à faire sa propre extraction.
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
import sys
import subprocess

from module.logger_app import log


def env_relance() -> dict:
    """Copie de l'environnement débarrassée des variables internes PyInstaller.

    Sans effet en mode dev (ces variables n'existent pas). En mode frozen
    --onefile, garantit que l'instance relancée extrait son propre `_MEIxxxxx`.
    """
    env = dict(os.environ)
    for cle in list(env.keys()):
        if cle == "_MEIPASS2" or cle.startswith("_PYI"):
            env.pop(cle, None)
    return env


def args_relance() -> list:
    """Arguments pour relancer l'application (sans flag particulier)."""
    if getattr(sys, "frozen", False):
        return [sys.executable]
    # __file__ = module/utilitaire/redemarrage.py → racine = 2 niveaux au-dessus
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.dirname(os.path.dirname(here))
    return [sys.executable, os.path.join(root, "main.py")]


def redemarrer(root=None) -> bool:
    """Lance une nouvelle instance puis ferme l'instance courante.

    `root` : la fenêtre racine Tk (obtenue via widget.winfo_toplevel()).
    Retourne False si le lancement de la nouvelle instance échoue (dans ce
    cas l'instance courante reste ouverte). Sinon ne retourne pas (l'appli
    se ferme).
    """
    try:
        subprocess.Popen(
            args_relance(),
            env=env_relance(),
            creationflags=(
                subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
            ),
        )
    except Exception as e:
        log.warning(f"redemarrer: lancement de la nouvelle instance échoué: {e}")
        return False

    # Ferme l'instance courante. destroy() termine le mainloop → main()
    # retourne → le process se termine (les threads workers sont daemon).
    if root is not None:
        try:
            root.destroy()
        except Exception:
            pass
    return True
