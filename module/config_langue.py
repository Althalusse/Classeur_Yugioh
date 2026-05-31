"""
config_langue.py — Gestion de la langue d'affichage des noms de cartes.

Sauvegarde le choix EN / FR dans bdd/app_config.json via module.app_config.
Utilisé par :
  - affichage_carte_classeur.py  (visualiseur binder)
  - inventaire_carte.py          (inventaire des possédées)
  - recherche_carte.py           (recherche dans les classeurs)

Langue par défaut : FR.

OPTIMISATION : cache en mémoire invalidé à chaque save_langue(), pour éviter
les appels répétés à app_config.get() lors de l'affichage des cartes.
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

import module.app_config as _cfg

_LANGUES_DISPONIBLES = ["EN", "FR"]
_LANGUE_DEFAUT       = "FR"

# Cache module-level — évite les lectures répétées pendant l'affichage
_langue_cache: str | None = None


def load_langue() -> str:
    """Charge la langue depuis app_config.json. Retourne 'FR' par défaut."""
    global _langue_cache
    if _langue_cache is not None:
        return _langue_cache
    langue = _cfg.get("langue", _LANGUE_DEFAUT)
    _langue_cache = langue if langue in _LANGUES_DISPONIBLES else _LANGUE_DEFAUT
    return _langue_cache


def save_langue(langue: str) -> None:
    """Sauvegarde la langue dans app_config.json et met à jour le cache."""
    global _langue_cache
    if langue not in _LANGUES_DISPONIBLES:
        langue = _LANGUE_DEFAUT
    _cfg.set("langue", langue)
    _langue_cache = langue  # mise à jour immédiate du cache


def get_name_column() -> str:
    """
    Retourne le nom de colonne SQL à utiliser selon la langue courante.
    EN → 'name'
    FR → 'name_fr' (avec fallback 'name' si name_fr est NULL/vide)
    """
    return "name_fr" if load_langue() == "FR" else "name"
