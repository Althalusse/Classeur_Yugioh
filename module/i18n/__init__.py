"""
i18n.py — Service de traduction de l'interface.

Usage :
    from module.i18n import t
    ttk.Button(text=t("btn.save"))
    ttk.Label(text=t("tab.statistics"))

La langue est lue depuis app_config.json (même fichier que config_langue.py).
Langue par défaut : FR.
Rechargement : redémarrer l'application après changement de langue UI.
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

import json
import os
import module.app_config as _cfg
from module.logger_app import log

_LANG_DEFAULT = "FR"
_SUPPORTED    = {"FR", "EN"}

# Chemins vers les fichiers de traduction
# En mode frozen PyInstaller --onedir :
#   sys._MEIPASS = dossier _internal/ → les JSON y sont copiés via --add-data
#   __file__ = _internal/module/i18n/__init__.py → os.path.dirname() = bon dossier
# En mode dev : __file__ = module/i18n/__init__.py → identique
# Les deux cas sont couverts par os.path.dirname(__file__)
_I18N_DIR = os.path.dirname(os.path.abspath(__file__))

_translations: dict = {}
_current_lang: str  = _LANG_DEFAULT
_initialized: bool  = False


def _ensure_loaded():
    """Charge les traductions FR par défaut si init() n'a pas encore été appelé."""
    global _translations, _initialized
    if not _initialized:
        _translations = _load("FR")
        _initialized = True


def _load(lang: str) -> dict:
    """Charge le fichier JSON de la langue donnée."""
    path = os.path.join(_I18N_DIR, f"{lang.lower()}.json")
    if not os.path.exists(path):
        return {}
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception as e:
        log.warning(f"i18n : impossible de charger {path} : {e}")
        return {}


def init():
    """
    Initialise le service i18n.
    Appelé une seule fois depuis main.py au démarrage,
    avant la création de la fenêtre principale.
    """
    global _translations, _current_lang, _initialized

    # Lit la langue UI depuis app_config.json via le module centralisé
    lang = _LANG_DEFAULT
    ui_lang = _cfg.get("ui_langue", _LANG_DEFAULT)
    if isinstance(ui_lang, str) and ui_lang.upper() in _SUPPORTED:
        lang = ui_lang.upper()

    _current_lang = lang

    # Charge FR en base, puis surcharge avec la langue choisie si différente
    _translations = _load("FR")
    if lang != "FR":
        overrides = _load(lang)
        _translations.update(overrides)

    _initialized = True
    log.info(f"Langue UI : {_current_lang} ({len(_translations)} clés)")


def t(key: str, **kwargs) -> str:
    """
    Retourne la traduction de la clé donnée.
    Si la clé est absente, retourne la clé elle-même (jamais de crash).
    Charge les traductions FR par défaut si init() n'a pas encore été appelé.
    """
    _ensure_loaded()
    text = _translations.get(key, key)
    if kwargs:
        try:
            text = text.format(**kwargs)
        except Exception:
            pass
    return text


def get_current_lang() -> str:
    """Retourne la langue UI actuellement chargée."""
    return _current_lang


def save_ui_langue(lang: str) -> None:
    """
    Sauvegarde la langue UI dans app_config.json.
    Le changement prend effet au prochain démarrage.
    """
    lang = lang.upper()
    if lang not in _SUPPORTED:
        lang = _LANG_DEFAULT

    _cfg.set("ui_langue", lang)
