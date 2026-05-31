"""
logger_app.py — Système de log centralisé pour l'application.

Écrit tout dans logs/app.log (à côté du .exe / de main.py) :
- Messages INFO : démarrage, étapes clés
- Exceptions : avec traceback complet
- Erreurs Tkinter silencieuses (via sys.excepthook + Tk.report_callback_exception)

Usage :
    from module.logger_app import log, install_handlers
    log.info("Démarrage")
    log.exception("Oups")  # capture automatiquement le traceback
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

import logging
import logging.handlers
import os
import sys
import traceback
from datetime import datetime

# ─────────────────────────────────────────────────────────────────────────────
# Chemin du dossier logs — robuste frozen / dev
# ─────────────────────────────────────────────────────────────────────────────

def _root_dir() -> str:
    if getattr(sys, "frozen", False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


_LOG_DIR  = os.path.join(_root_dir(), "logs")
_LOG_FILE = os.path.join(_LOG_DIR, "app.log")

os.makedirs(_LOG_DIR, exist_ok=True)

# ─────────────────────────────────────────────────────────────────────────────
# Logger principal
# ─────────────────────────────────────────────────────────────────────────────

log = logging.getLogger("yugioh_manager")
log.setLevel(logging.DEBUG)

# Éviter doublon de handlers si le module est rechargé
if not log.handlers:
    fmt = logging.Formatter(
        fmt="%(asctime)s  [%(levelname)-5s]  %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )

    # Fichier — rotatif : plafonne la taille du log (1 Mo par fichier,
    # 3 sauvegardes → app.log + app.log.1..3 = ~4 Mo max au total), au lieu
    # d'une croissance illimitée. La rotation se fait automatiquement quand
    # app.log atteint maxBytes.
    try:
        fh = logging.handlers.RotatingFileHandler(
            _LOG_FILE, maxBytes=1_000_000, backupCount=3, encoding="utf-8"
        )
    except Exception:
        # Repli : si la rotation échoue (ex. fichier verrouillé par un autre
        # process sous Windows), on retombe sur un FileHandler simple plutôt
        # que de perdre toute journalisation.
        fh = logging.FileHandler(_LOG_FILE, encoding="utf-8")
    fh.setLevel(logging.DEBUG)
    fh.setFormatter(fmt)
    log.addHandler(fh)

    # Console (stdout) — pour dev
    try:
        sh = logging.StreamHandler(sys.stdout)
        sh.setLevel(logging.INFO)
        sh.setFormatter(fmt)
        log.addHandler(sh)
    except Exception:
        pass


def install_handlers(root=None):
    """
    Installe les hooks globaux pour capturer les exceptions non gérées.
    À appeler UNE FOIS au démarrage, après création de root.

    root : ctk.CTk / tk.Tk — pour capturer les exceptions des callbacks Tkinter.
    """

    # 1) Exceptions Python non gérées (hors Tkinter)
    def _excepthook(exc_type, exc_value, exc_tb):
        if issubclass(exc_type, KeyboardInterrupt):
            sys.__excepthook__(exc_type, exc_value, exc_tb)
            return
        log.critical(
            "UNHANDLED EXCEPTION:\n%s",
            "".join(traceback.format_exception(exc_type, exc_value, exc_tb))
        )

    sys.excepthook = _excepthook

    # 2) Exceptions dans les callbacks Tkinter (après, bind, ...)
    if root is not None:
        def _tk_exc_handler(exc_type, exc_value, exc_tb):
            log.error(
                "TKINTER CALLBACK EXCEPTION:\n%s",
                "".join(traceback.format_exception(exc_type, exc_value, exc_tb))
            )
        try:
            root.report_callback_exception = _tk_exc_handler
        except Exception as e:
            log.warning("Impossible d'installer report_callback_exception: %s", e)

    log.info("=" * 70)
    log.info("Session démarrée — %s", datetime.now().isoformat(timespec="seconds"))
    log.info("Python : %s", sys.version.split()[0])
    log.info("Platform : %s", sys.platform)
    log.info("Log file : %s", _LOG_FILE)
    log.info("=" * 70)


def get_log_path() -> str:
    """Retourne le chemin absolu du fichier de log (utile pour l'afficher à l'utilisateur)."""
    return _LOG_FILE
