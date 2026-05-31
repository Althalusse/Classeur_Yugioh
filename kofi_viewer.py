"""
kofi_viewer.py — Affichage de la page Ko-fi dans une fenêtre pywebview.

Importé par main.py quand l'application est relancée avec le flag --kofi.
Ce process dédié n'a PAS de mainloop Tkinter : pywebview peut donc tourner
seul sur son thread principal (le conflit thread principal Tkinter/pywebview
disparaît). Compatible PyInstaller --onefile : l'app se relance elle-même,
donc UN SEUL .exe et aucun fichier externe à embarquer.

run() : ouvre la fenêtre Ko-fi. En l'absence de pywebview / WebView2, ou en
cas d'erreur quelconque, ouvre Ko-fi dans le navigateur (fallback sûr).
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

import sys
import os
import tempfile
import base64

KOFI_URL = (
    "https://ko-fi.com/althalusse/?hidefeed=true&widget=true&embed=true&preview=true"
)
KOFI_PAGE = "https://ko-fi.com/althalusse"
_TITRE = "\u2615  Offre un Caf\u00e9 \u00e0 Althalusse \u2014 Ko-fi"

# Ic\u00f4ne \u2615 encod\u00e9e en base64 \u2014 extraite dans un fichier temp au lancement
_ICO_B64 = "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"


def _extract_icon():
    """\u00c9crit l'ic\u00f4ne dans un fichier temporaire et retourne son chemin."""
    try:
        data = base64.b64decode(_ICO_B64)
        tmp = tempfile.NamedTemporaryFile(suffix=".ico", delete=False)
        tmp.write(data)
        tmp.close()
        return tmp.name
    except Exception:
        return None


def _ouvrir_navigateur():
    try:
        import webbrowser
        webbrowser.open(KOFI_PAGE)
    except Exception:
        pass


def run():
    """Ouvre la fen\u00eatre Ko-fi (pywebview) ou le navigateur en fallback."""
    try:
        import webview
    except ImportError:
        _ouvrir_navigateur()
        return

    icon_path = None
    try:
        icon_path = _extract_icon()
        webview.create_window(
            _TITRE, KOFI_URL, width=520, height=760, resizable=True,
        )

        def _set_icon():
            """Applique l'ic\u00f4ne via la fen\u00eatre Win32 sous-jacente."""
            if not icon_path or sys.platform != "win32":
                return
            try:
                import ctypes
                IMAGE_ICON = 1
                LR_LOADFROMFILE = 0x00000010
                ICON_SMALL = 0
                ICON_BIG = 1
                WM_SETICON = 0x0080
                hIcon = ctypes.windll.user32.LoadImageW(
                    None, icon_path, IMAGE_ICON, 0, 0, LR_LOADFROMFILE
                )
                if hIcon:
                    import win32gui  # type: ignore \u2014 optionnel
                    hwnd = win32gui.FindWindow(None, _TITRE)
                    if hwnd:
                        win32gui.SendMessage(hwnd, WM_SETICON, ICON_SMALL, hIcon)
                        win32gui.SendMessage(hwnd, WM_SETICON, ICON_BIG, hIcon)
            except Exception:
                pass

        webview.start(_set_icon)
    except Exception:
        _ouvrir_navigateur()
    finally:
        try:
            if icon_path and os.path.exists(icon_path):
                os.unlink(icon_path)
        except Exception:
            pass


if __name__ == "__main__":
    run()
