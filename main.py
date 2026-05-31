"""
main.py — Point d'entrée de l'application Yu-Gi-Oh! Collection Manager.

Correction du bug Windows :
Au lieu de root.withdraw() + show_init_window() + mainloop(), on appelle
show_init_window() DEPUIS la mainloop principale via after_idle(). Comme ça
la racine reste vivante et mainloop() ne retourne pas prématurément quand
le Toplevel d'init est détruit.
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
import traceback

if not getattr(sys, 'frozen', False):
    _root = os.path.abspath(os.path.dirname(__file__))
    if _root not in sys.path:
        sys.path.insert(0, _root)


try:
    from module.logger_app import log, install_handlers, get_log_path
except Exception as e:
    _log_dir = os.path.join(
        os.path.dirname(os.path.abspath(sys.executable if getattr(sys, 'frozen', False) else __file__)),
        "logs",
    )
    os.makedirs(_log_dir, exist_ok=True)
    with open(os.path.join(_log_dir, "app.log"), "a", encoding="utf-8") as f:
        f.write(f"\n[CRITICAL] Impossible d'importer le logger : {e}\n")
        f.write(traceback.format_exc())
    raise


def _show_fatal_error(message: str):
    try:
        import tkinter as tk
        from tkinter import messagebox
        r = tk.Tk()
        r.withdraw()
        messagebox.showerror("Erreur critique", message)
        r.destroy()
    except Exception:
        print("FATAL:", message, file=sys.stderr)


def _safe_main():
    log.info("Démarrage de l'application")

    try:
        import customtkinter as ctk
        log.info("customtkinter %s", getattr(ctk, "__version__", "?"))
    except Exception:
        log.exception("customtkinter indisponible")
        _show_fatal_error("customtkinter n'est pas installé.\n\n"
                          "pip install customtkinter Pillow requests ratelimit")
        return

    try:
        from module.centralisation_dossier import (
            FIRST_RUN_FILE, CLASSEUR_FOLDER, init_folders, sqlite_ctx,
        )
        from module.theme import setup_ctk
        from module.ui.init_window import show_init_window
        from module.ui.app_window import build_app
        from module.db_migrations import ensure_columns
        from module import i18n
        from module.i18n import t
    except Exception:
        log.exception("Erreur lors des imports applicatifs")
        _show_fatal_error(f"Erreur au chargement des modules.\nVoir : {get_log_path()}")
        return

    try:
        init_folders()
        i18n.init()
        setup_ctk()
    except Exception:
        log.exception("Erreur dans init_folders / i18n / setup_ctk")
        _show_fatal_error(f"Erreur au démarrage.\nVoir : {get_log_path()}")
        return

    # ═════════════════════════════════════════════════════════════════════
    # PHASE 1 : Initialisation BDD (si premier lancement)
    # ═════════════════════════════════════════════════════════════════════
    # Effectuée AVANT la création de la fenêtre principale pour éviter une
    # course entre root.state("zoomed") et show_init_window() : sans ça,
    # la Toplevel d'init se retrouvait masquée par la maximisation de la
    # racine CTk en cours, et l'utilisateur ne voyait rien pendant les
    # ~9-10 secondes de téléchargement YGOJSON.
    #
    # On utilise une mini-racine tk.Tk() DÉDIÉE — invisible (withdraw) mais
    # vivante — pour servir de parent. Elle est détruite après l'init pour
    # laisser la vraie racine CTk prendre le relais proprement.
    # ═════════════════════════════════════════════════════════════════════
    if not os.path.exists(FIRST_RUN_FILE):
        log.info("Premier lancement — initialisation BDD")
        try:
            import tkinter as _tk
            init_root = _tk.Tk()
            init_root.withdraw()
            init_root.title("")

            show_init_window()

            try:
                with open(FIRST_RUN_FILE, "w") as f:
                    f.write("initialized")
            except Exception:
                log.exception("Écriture first_run.flag impossible")

            try:
                init_root.destroy()
            except Exception:
                pass
            log.info("Initialisation BDD terminée")
        except Exception:
            log.exception("Erreur PHASE 1 init BDD")
            # On continue quand même : l'app peut démarrer en mode dégradé

    # ═════════════════════════════════════════════════════════════════════
    # PHASE 2 : Création fenêtre principale + UI
    # ═════════════════════════════════════════════════════════════════════
    try:
        root = ctk.CTk()
        install_handlers(root)
        try:
            root.title(t("app.window_title"))
        except Exception:
            root.title("Yu-Gi-Oh! Collection Manager")
        root.minsize(1024, 700)

        try:
            root.state("zoomed")
        except Exception:
            try:
                root.attributes("-zoomed", True)
            except Exception:
                root.geometry("1280x800")
    except Exception:
        log.exception("Erreur création racine CTk")
        _show_fatal_error(f"Erreur fenêtre principale.\nVoir : {get_log_path()}")
        return

    # ── Migrations des classeurs existants ────────────────────────────────
    #
    # Migration séquentielle volontaire (pas de thread) : les ALTER TABLE
    # concurrents sur différents classeurs partagent les pragma WAL et
    # pourraient entrer en conflit. Pour <200 classeurs l'impact est
    # négligeable (~20-40 ms par classeur).
    #
    # Les échecs sont accumulés plutôt que swallow silencieusement (M7) :
    # l'utilisateur est averti si un classeur n'a pas pu être migré,
    # au lieu de découvrir plus tard une erreur cryptique "no such column".
    migrated = 0
    failed: list[tuple[str, str]] = []  # liste de (code_classeur, message)
    try:
        if os.path.isdir(CLASSEUR_FOLDER):
            for name in os.listdir(CLASSEUR_FOLDER):
                db = os.path.join(CLASSEUR_FOLDER, name, f"{name}.db")
                if not os.path.exists(db):
                    continue
                try:
                    with sqlite_ctx(db) as conn:
                        ensure_columns(conn)
                    migrated += 1
                except Exception as exc:
                    log.exception("Migration échouée pour %s", db)
                    failed.append((name, str(exc)))
        log.info("Migrations classeurs : %d OK, %d échec(s)", migrated, len(failed))
    except Exception:
        log.exception("Erreur globale pendant les migrations")

    # Avertissement utilisateur visible en cas d'échec(s)
    if failed:
        try:
            from tkinter import messagebox
            preview = "\n".join(f"  • {n} : {m[:80]}" for n, m in failed[:5])
            reste = len(failed) - 5
            if reste > 0:
                preview += f"\n  … et {reste} autre(s)"
            messagebox.showwarning(
                "Classeurs non migrés",
                "Certains classeurs n'ont pas pu être mis à jour au démarrage.\n"
                "Ils risquent d'afficher des erreurs à l'ouverture.\n\n"
                f"Détails ({len(failed)} classeur(s) concerné(s)) :\n{preview}\n\n"
                f"Journal complet : {get_log_path()}"
            )
        except Exception:
            log.exception("Impossible d'afficher l'avertissement migration")

    # ── Construction de l'UI ──────────────────────────────────────────────
    try:
        build_app(root)
        log.info("UI construite")
    except Exception:
        log.exception("Erreur dans build_app")
        try:
            root.destroy()
        except Exception:
            pass
        _show_fatal_error(
            f"Erreur à la construction de l'interface.\nVoir : {get_log_path()}"
        )
        return

    try:
        root.mainloop()
    except Exception:
        log.exception("Exception dans mainloop")

    log.info("Application fermée normalement")


def main():
    try:
        _safe_main()
    except SystemExit:
        raise
    except BaseException:
        log.critical("EXCEPTION TOP-LEVEL:\n%s", traceback.format_exc())
        _show_fatal_error(
            f"Erreur fatale non gérée.\nVoir : {get_log_path()}"
        )


if __name__ == "__main__":
    # Mode viewer Ko-fi : l'écran Contribution relance l'application avec
    # --kofi. Ce process dédié n'a PAS de mainloop Tkinter, donc pywebview
    # tourne seul sur son thread principal (résout le conflit Tkinter/pywebview).
    # Compatible PyInstaller --onefile : un seul .exe qui se relance lui-même.
    # Doit court-circuiter AVANT toute init lourde (BDD, i18n, fenêtre).
    if "--kofi" in sys.argv:
        try:
            import kofi_viewer
            kofi_viewer.run()
        except Exception:
            try:
                import webbrowser
                webbrowser.open("https://ko-fi.com/althalusse")
            except Exception:
                pass
        sys.exit(0)

    main()
