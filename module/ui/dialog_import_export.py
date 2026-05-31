"""
dialog_import_export.py — Fenêtre CTk d'import/export CSV format Scanflip.

Pendant à update_window.py (UpdateWindow CTk), centralise toutes les
opérations d'import/export pour la collection.

Architecture :
  - Toplevel CTk avec deux onglets internes (Tabview) : "📤 Export" et "📥 Import"
  - Choix du scope :
      * Toute la collection (tous les classeurs)
      * Un classeur précis (dropdown rempli depuis CLASSEUR_FOLDER)
  - Pour l'import : sélecteur de fichier CSV + résumé après opération

Modale tant qu'aucune opération longue n'est en cours.

Pré-configuration possible :
    DialogImportExport(parent, classeur_initial="RA02", onglet_initial="export")
  → ouvre directement sur l'onglet Export avec RA02 sélectionné.
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
import threading
import tkinter as tk
import tkinter.filedialog as fd
import tkinter.messagebox as mb
import customtkinter as ctk

from module.theme import C
from module.ui.composants import gold_button, secondary_button
from module.export.export_collection import (
    exporter_csv, get_classeurs_disponibles,
)
from module.import_csv.import_collection import (
    importer_csv, detecter_classeurs_absents,
)
from module.logger_app import log


# Constantes UI
W_WINDOW = 720
H_WINDOW = 580

LABEL_TOUS_CLASSEURS = "— Toute la collection —"


class DialogImportExport(ctk.CTkToplevel):
    """
    Fenêtre modale CTk d'import/export Scanflip.

    Args:
        parent             : widget parent (typiquement la racine CTk).
        classeur_initial   : code de classeur à pré-sélectionner (None = tous).
        onglet_initial     : "export" ou "import" — onglet à afficher au démarrage.
        on_update          : callback appelé après un import réussi (pour
                              rafraîchir l'UI appelante — ex le visualiseur).
    """

    def __init__(self, parent,
                 classeur_initial: str | None = None,
                 onglet_initial: str = "export",
                 on_update=None):
        super().__init__(parent)
        self.title("Import / Export CSV")
        self.configure(fg_color=C["bg"])
        self.resizable(False, False)

        # Centrage écran
        try:
            sw = parent.winfo_screenwidth()
            sh = parent.winfo_screenheight()
        except Exception:
            sw, sh = 1920, 1080
        x = (sw - W_WINDOW) // 2
        y = (sh - H_WINDOW) // 2
        self.geometry(f"{W_WINDOW}x{H_WINDOW}+{x}+{y}")

        try:
            self.transient(parent)
            self.grab_set()
        except Exception:
            pass

        self._on_update_cb = on_update
        self._destroyed    = False
        self._operation_en_cours = False

        # ─── Containers ───────────────────────────────────────────────────
        self._container = ctk.CTkFrame(self, fg_color="transparent")
        self._container.pack(fill="both", expand=True, padx=24, pady=20)

        ctk.CTkLabel(
            self._container,
            text="📋  Import / Export de la collection",
            font=("Outfit", 16, "bold"),
            text_color=C["text"],
        ).pack(anchor="w", pady=(0, 4))
        ctk.CTkLabel(
            self._container,
            text="Format Scanflip — compatible round-trip",
            font=("Outfit", 11),
            text_color=C["text3"],
        ).pack(anchor="w", pady=(0, 16))

        # Tabview avec onglets Export et Import
        self._tabs = ctk.CTkTabview(
            self._container,
            fg_color=C["bg2"],
            segmented_button_fg_color=C["bg3"],
            segmented_button_selected_color=C["gold"],
            segmented_button_selected_hover_color=C["gold_hover"],
            segmented_button_unselected_color=C["bg3"],
            segmented_button_unselected_hover_color=C["border2"],
            text_color=C["text"],
        )
        self._tabs.pack(fill="both", expand=True)

        self._tab_export = self._tabs.add("📤  Export")
        self._tab_import = self._tabs.add("📥  Import")

        # Pré-sélection de l'onglet
        try:
            self._tabs.set("📤  Export" if onglet_initial == "export" else "📥  Import")
        except Exception:
            pass

        # Construction des contenus de chaque onglet
        self._build_export_tab(classeur_initial)
        self._build_import_tab(classeur_initial)

        # Cycle de vie
        self.protocol("WM_DELETE_WINDOW", self._on_close)
        self.bind("<Escape>", lambda e: self._on_close())

    # ═══════════════════════════════════════════════════════════════════════
    # Onglet EXPORT
    # ═══════════════════════════════════════════════════════════════════════

    def _build_export_tab(self, classeur_initial: str | None):
        tab = self._tab_export
        inner = ctk.CTkFrame(tab, fg_color="transparent")
        inner.pack(fill="both", expand=True, padx=16, pady=16)

        # Titre de section
        ctk.CTkLabel(
            inner,
            text="Exporter la collection vers un fichier CSV",
            font=("Outfit", 13, "bold"),
            text_color=C["text"],
        ).pack(anchor="w", pady=(0, 16))

        # ─── Choix du scope ───────────────────────────────────────────────
        scope_frame = ctk.CTkFrame(inner, fg_color=C["bg3"], corner_radius=4)
        scope_frame.pack(fill="x", pady=(0, 16))
        scope_inner = ctk.CTkFrame(scope_frame, fg_color="transparent")
        scope_inner.pack(fill="x", padx=16, pady=12)

        ctk.CTkLabel(
            scope_inner,
            text="Que voulez-vous exporter ?",
            font=("Outfit", 11),
            text_color=C["text2"],
        ).pack(anchor="w", pady=(0, 8))

        classeurs = get_classeurs_disponibles()
        valeurs_dropdown = [LABEL_TOUS_CLASSEURS] + classeurs

        self._export_classeur_var = ctk.StringVar(value=LABEL_TOUS_CLASSEURS)
        # Pré-sélection si classeur_initial fourni et présent
        if classeur_initial and classeur_initial in classeurs:
            self._export_classeur_var.set(classeur_initial)

        self._export_dropdown = ctk.CTkOptionMenu(
            scope_inner,
            values=valeurs_dropdown,
            variable=self._export_classeur_var,
            fg_color=C["bg2"],
            button_color=C["gold"],
            button_hover_color=C["gold_hover"],
            text_color=C["text"],
            dropdown_fg_color=C["bg2"],
            width=320,
        )
        self._export_dropdown.pack(anchor="w")

        # ─── Choix de la langue ───────────────────────────────────────────
        lang_frame = ctk.CTkFrame(inner, fg_color=C["bg3"], corner_radius=4)
        lang_frame.pack(fill="x", pady=(0, 16))
        lang_inner = ctk.CTkFrame(lang_frame, fg_color="transparent")
        lang_inner.pack(fill="x", padx=16, pady=12)

        ctk.CTkLabel(
            lang_inner,
            text="Langue d'export",
            font=("Outfit", 11),
            text_color=C["text2"],
        ).pack(anchor="w", pady=(0, 8))

        self._export_langue_var = ctk.StringVar(value="FR")
        lang_radio_frame = ctk.CTkFrame(lang_inner, fg_color="transparent")
        lang_radio_frame.pack(anchor="w")
        for label, val in [("Français", "FR"), ("English", "EN")]:
            ctk.CTkRadioButton(
                lang_radio_frame,
                text=label,
                variable=self._export_langue_var,
                value=val,
                fg_color=C["gold"],
                hover_color=C["gold_hover"],
                border_color=C["gold_dim"],
                border_width_unchecked=2,
                text_color=C["text"],
            ).pack(side="left", padx=(0, 16))

        # ─── Statut + Bouton d'action ─────────────────────────────────────
        self._export_status = ctk.CTkLabel(
            inner,
            text="",
            font=("Outfit", 11),
            text_color=C["text2"],
            wraplength=W_WINDOW - 80,
            justify="left",
        )
        self._export_status.pack(anchor="w", pady=(8, 0))

        actions = ctk.CTkFrame(inner, fg_color="transparent")
        actions.pack(fill="x", pady=(16, 0))
        secondary_button(actions, "Fermer", command=self._on_close).pack(side="left")
        gold_button(actions, "📤  Exporter…", command=self._action_export).pack(side="right")

    def _action_export(self):
        if self._operation_en_cours:
            return
        # Sélection du classeur
        choice = self._export_classeur_var.get()
        classeur = None if choice == LABEL_TOUS_CLASSEURS else choice
        langue = self._export_langue_var.get()

        # Sélecteur de fichier
        default_name = (
            f"{classeur}_collection.csv" if classeur else "collection_complete.csv"
        )
        chemin = fd.asksaveasfilename(
            parent=self,
            title="Exporter en CSV",
            defaultextension=".csv",
            filetypes=[("CSV (Scanflip)", "*.csv"), ("Tous les fichiers", "*.*")],
            initialfile=default_name,
        )
        if not chemin:
            return  # annulation utilisateur

        # Statut "en cours"
        self._set_export_status("⏳ Export en cours…", C["text2"])
        self._operation_en_cours = True

        # Lancement en thread pour ne pas figer l'UI
        threading.Thread(
            target=self._export_worker,
            args=(chemin, langue, classeur),
            daemon=True,
        ).start()

        # Libère le grab pendant l'opération : l'utilisateur peut interagir
        # avec le centre d'activité et le reste de l'app. Ré-acquis sur retour.
        self._release_grab_pendant_operation()

    def _export_worker(self, chemin: str, langue: str, classeur):
        try:
            result = exporter_csv(chemin, langue=langue, classeur=classeur)
            self._safe_after(0, self._on_export_done, result)
        except Exception as e:
            self._safe_after(0, self._on_export_error, e)

    def _on_export_done(self, result: dict):
        self._operation_en_cours = False
        self._reacquerir_grab_post_operation()
        scope = (
            f"classeur {result['classeurs_exportes'][0]}"
            if len(result['classeurs_exportes']) == 1 else
            f"{len(result['classeurs_exportes'])} classeur(s)"
        )
        msg = (
            f"✅ {result['total_cartes']} carte(s) exportée(s) depuis {scope}.\n"
            f"Fichier : {result['chemin']}"
        )
        self._set_export_status(msg, C["gold"])

    def _on_export_error(self, exc: Exception):
        self._operation_en_cours = False
        self._reacquerir_grab_post_operation()
        self._set_export_status(
            f"❌ Erreur d'export : {exc}",
            C["danger"],
        )

    def _set_export_status(self, text: str, color: str):
        if self._destroyed:
            return
        try:
            self._export_status.configure(text=text, text_color=color)
        except Exception:
            pass

    # ═══════════════════════════════════════════════════════════════════════
    # Onglet IMPORT
    # ═══════════════════════════════════════════════════════════════════════

    def _build_import_tab(self, classeur_initial: str | None):
        tab = self._tab_import
        inner = ctk.CTkFrame(tab, fg_color="transparent")
        inner.pack(fill="both", expand=True, padx=16, pady=16)

        ctk.CTkLabel(
            inner,
            text="Importer une collection depuis un fichier CSV",
            font=("Outfit", 13, "bold"),
            text_color=C["text"],
        ).pack(anchor="w", pady=(0, 16))

        # ─── Sélection fichier ────────────────────────────────────────────
        file_frame = ctk.CTkFrame(inner, fg_color=C["bg3"], corner_radius=4)
        file_frame.pack(fill="x", pady=(0, 12))
        file_inner = ctk.CTkFrame(file_frame, fg_color="transparent")
        file_inner.pack(fill="x", padx=16, pady=12)

        ctk.CTkLabel(
            file_inner,
            text="Fichier CSV à importer",
            font=("Outfit", 11),
            text_color=C["text2"],
        ).pack(anchor="w", pady=(0, 8))

        path_row = ctk.CTkFrame(file_inner, fg_color="transparent")
        path_row.pack(fill="x")

        self._import_file_path = ctk.StringVar(value="")
        path_label = ctk.CTkLabel(
            path_row,
            textvariable=self._import_file_path,
            font=("Consolas", 10),
            text_color=C["text3"],
            anchor="w",
            wraplength=W_WINDOW - 200,
            justify="left",
        )
        path_label.pack(side="left", fill="x", expand=True, padx=(0, 8))

        secondary_button(
            path_row, "Parcourir…",
            command=self._action_browse,
        ).pack(side="right")

        # ─── Choix du scope d'application ─────────────────────────────────
        scope_frame = ctk.CTkFrame(inner, fg_color=C["bg3"], corner_radius=4)
        scope_frame.pack(fill="x", pady=(0, 12))
        scope_inner = ctk.CTkFrame(scope_frame, fg_color="transparent")
        scope_inner.pack(fill="x", padx=16, pady=12)

        ctk.CTkLabel(
            scope_inner,
            text="Cible de l'import",
            font=("Outfit", 11),
            text_color=C["text2"],
        ).pack(anchor="w", pady=(0, 8))

        classeurs = get_classeurs_disponibles()
        valeurs_dropdown = [LABEL_TOUS_CLASSEURS] + classeurs

        self._import_classeur_var = ctk.StringVar(value=LABEL_TOUS_CLASSEURS)
        if classeur_initial and classeur_initial in classeurs:
            self._import_classeur_var.set(classeur_initial)

        self._import_dropdown = ctk.CTkOptionMenu(
            scope_inner,
            values=valeurs_dropdown,
            variable=self._import_classeur_var,
            fg_color=C["bg2"],
            button_color=C["gold"],
            button_hover_color=C["gold_hover"],
            text_color=C["text"],
            dropdown_fg_color=C["bg2"],
            width=320,
        )
        self._import_dropdown.pack(anchor="w")

        # ─── Mention sur le mode REPLACE ──────────────────────────────────
        ctk.CTkLabel(
            inner,
            text=("ℹ La quantité du CSV remplace celle de la base. "
                  "Les classeurs et cartes absents du CSV ne sont pas modifiés."),
            font=("Outfit", 10, "italic"),
            text_color=C["text3"],
            wraplength=W_WINDOW - 80,
            justify="left",
        ).pack(anchor="w", pady=(0, 8))

        # ─── Zone de résultat (cachée tant qu'aucun import) ───────────────
        self._import_result_frame = ctk.CTkFrame(
            inner, fg_color=C["bg3"], corner_radius=4,
        )
        # Pas de pack ici — apparaît au premier import

        # ─── Statut + Bouton d'action ─────────────────────────────────────
        self._import_status = ctk.CTkLabel(
            inner,
            text="",
            font=("Outfit", 11),
            text_color=C["text2"],
            wraplength=W_WINDOW - 80,
            justify="left",
        )
        self._import_status.pack(anchor="w", pady=(8, 0))

        actions = ctk.CTkFrame(inner, fg_color="transparent")
        actions.pack(fill="x", pady=(16, 0))
        secondary_button(actions, "Fermer", command=self._on_close).pack(side="left")
        self._btn_import = gold_button(
            actions, "📥  Importer", command=self._action_import,
        )
        self._btn_import.pack(side="right")

    def _action_browse(self):
        chemin = fd.askopenfilename(
            parent=self,
            title="Choisir un fichier CSV",
            filetypes=[("CSV (Scanflip)", "*.csv"), ("Tous les fichiers", "*.*")],
        )
        if chemin:
            self._import_file_path.set(chemin)
            self._set_import_status("", C["text2"])

    def _action_import(self):
        if self._operation_en_cours:
            return
        chemin = self._import_file_path.get()
        if not chemin:
            self._set_import_status(
                "⚠ Sélectionnez d'abord un fichier CSV.",
                C["danger"],
            )
            return
        if not os.path.exists(chemin):
            self._set_import_status(
                f"⚠ Fichier introuvable : {chemin}",
                C["danger"],
            )
            return

        choice   = self._import_classeur_var.get()
        classeur = None if choice == LABEL_TOUS_CLASSEURS else choice

        # ── Pré-check : classeurs absents en local ───────────────────────
        # Détection légère (lecture rapide du CSV, juste les Extensions).
        # Si l'utilisateur a filtré sur un classeur précis, on ne propose
        # pas de création (il sait ce qu'il fait, et le filtre signifie
        # "j'agis seulement sur ce classeur déjà présent").
        creer_classeurs_absents = False
        if classeur is None:
            try:
                absents = detecter_classeurs_absents(chemin)
            except Exception:
                absents = []

            if absents:
                # Demande de confirmation avec messagebox tkinter standard
                # (3 boutons via askyesnocancel).
                lignes = "\n".join(f"  • {code}" for code in absents[:15])
                if len(absents) > 15:
                    lignes += f"\n  … et {len(absents) - 15} autre(s)"
                msg = (
                    f"Les classeurs suivants sont présents dans le CSV mais "
                    f"absents en local :\n\n{lignes}\n\n"
                    f"Voulez-vous les créer automatiquement avant l'import ?\n\n"
                    f"• Oui   → création depuis cardinfo.db (instantané) "
                    f"puis import\n"
                    f"• Non   → import sans création (les lignes concernées "
                    f"seront ignorées)\n"
                    f"• Annuler → ne rien faire"
                )
                # askyesnocancel : True/False/None
                reponse = mb.askyesnocancel(
                    "Classeurs absents",
                    msg,
                    parent=self,
                )
                if reponse is None:
                    return  # Annulation
                creer_classeurs_absents = bool(reponse)

        # ── Lancement effectif ────────────────────────────────────────────
        self._set_import_status("⏳ Import en cours…", C["text2"])
        self._operation_en_cours = True

        threading.Thread(
            target=self._import_worker,
            args=(chemin, classeur, creer_classeurs_absents),
            daemon=True,
        ).start()

        # Libère le grab pendant l'opération : l'utilisateur peut interagir
        # avec le centre d'activité (suivre les créations de classeurs) et
        # le reste de l'app pendant que l'import tourne en arrière-plan.
        # Ré-acquis dans _on_import_done / _on_import_error.
        self._release_grab_pendant_operation()

    def _import_worker(self, chemin: str, classeur, creer_classeurs_absents: bool):
        try:
            # Si on doit créer des classeurs, on passe par FileAttenteClasseur
            # via un callback bloquant. Cela rend les créations visibles
            # dans le centre d'activité (au lieu d'être totalement
            # silencieuses comme avant). Le callback ajoute la tâche à
            # la file et attend qu'elle soit terminée avant de retourner.
            #
            # Pour `creer_classeurs_absents=False`, on ne fournit pas le
            # callback : aucune création ne sera tentée de toute façon.
            callback = self._creer_classeur_via_file if creer_classeurs_absents else None

            result = importer_csv(
                chemin,
                classeur_filtre=classeur,
                creer_classeurs_absents=creer_classeurs_absents,
                creer_classeur_callback=callback,
            )
            self._safe_after(0, self._on_import_done, result)
        except Exception as e:
            self._safe_after(0, self._on_import_error, e)

    def _creer_classeur_via_file(self, code_classeur: str):
        """Callback de création via FileAttenteClasseur (bloquant).

        Pousse la tâche dans le singleton, attend qu'elle soit terminée,
        puis retourne (succes, raison) pour `importer_csv()`.

        Le timeout est généreux (5 min) car la création peut inclure un
        appel API YGOPRODeck — typiquement <30 s mais on laisse de la
        marge en cas de réseau lent ou de set très volumineux. Au-delà
        on considère que quelque chose est cassé et on rapporte un échec
        plutôt que de bloquer l'import indéfiniment.

        IMPORTANT : appelé depuis le thread `_import_worker`, donc le
        wait() est sans danger pour l'UI principale.
        """
        try:
            from module.img_dl.file_attente_classeur import (
                FileAttenteClasseur, StatutTache,
            )
            file = FileAttenteClasseur()
            tache = file.ajouter(code_classeur)
            # Attente bloquante (5 min max). file.attendre_taches gère le
            # timeout cumulé proprement, pas besoin de boucle manuelle.
            ok = file.attendre_taches([tache], timeout=300)
            if not ok:
                return (False, "Timeout (création > 5 min — abandon).")
            if tache.statut == StatutTache.TERMINE:
                return (True, None)
            # ANNULE / ERREUR — on retourne le message du worker.
            return (False, tache.message or f"Statut final {tache.statut.name}")
        except Exception as e:
            return (False, f"Exception callback : {e}")

    def _on_import_done(self, result: dict):
        self._operation_en_cours = False
        self._reacquerir_grab_post_operation()

        # Affiche la zone de résumé (pack si pas encore visible)
        try:
            self._import_result_frame.pack(fill="x", pady=(8, 0), before=self._import_status)
        except Exception:
            pass
        # Vide les anciens résultats
        for w in self._import_result_frame.winfo_children():
            w.destroy()

        inner = ctk.CTkFrame(self._import_result_frame, fg_color="transparent")
        inner.pack(fill="x", padx=16, pady=12)

        ctk.CTkLabel(
            inner,
            text="✅ Import terminé",
            font=("Outfit", 12, "bold"),
            text_color=C["gold"],
        ).pack(anchor="w", pady=(0, 8))

        # Statistiques
        for label, value, color in [
            ("Lignes lues du CSV     :", result["total_lignes"],         C["text2"]),
            ("Cartes mises à jour    :", result["importees"],            C["gold"]),
            ("Lignes ignorées        :", result["ignorees"],             C["text3"]),
            ("Cartes non trouvées    :", result.get("non_trouvees_total", 0), C["text3"]),
            ("Avertissements         :", result.get("warnings_total", 0),    C["text3"]),
        ]:
            row = ctk.CTkFrame(inner, fg_color="transparent")
            row.pack(fill="x", pady=1)
            ctk.CTkLabel(
                row, text=label,
                font=("Outfit", 10), text_color=C["text3"], width=180, anchor="w",
            ).pack(side="left")
            ctk.CTkLabel(
                row, text=str(value),
                font=("Consolas", 10, "bold"), text_color=color,
            ).pack(side="left")

        # Classeurs créés (nouveauté Lot création-auto)
        if result.get("classeurs_crees"):
            ctk.CTkLabel(
                inner,
                text=("✨ Classeurs créés depuis cardinfo.db : "
                      f"{', '.join(result['classeurs_crees'])}"),
                font=("Outfit", 10),
                text_color=C["gold"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", pady=(8, 0))

        # Classeurs traités (UPDATE)
        if result.get("classeurs_traites"):
            ctk.CTkLabel(
                inner,
                text=f"Classeurs mis à jour : {', '.join(result['classeurs_traites'])}",
                font=("Outfit", 10),
                text_color=C["text2"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", pady=(4, 0))

        # Échecs de création (nouveauté)
        if result.get("classeurs_creation_echec"):
            echecs_lignes = []
            for entry in result["classeurs_creation_echec"][:5]:
                echecs_lignes.append(f"  • {entry['classeur']} : {entry['raison'][:60]}")
            reste = len(result["classeurs_creation_echec"]) - 5
            if reste > 0:
                echecs_lignes.append(f"  … et {reste} autre(s)")
            ctk.CTkLabel(
                inner,
                text="❌ Échec de création (set absent de cardinfo.db) :\n"
                     + "\n".join(echecs_lignes)
                     + "\n→ Lancez 'MAJ BDD' depuis l'accueil pour récupérer les sets récents.",
                font=("Outfit", 10),
                text_color=C["danger"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", pady=(4, 0))

        # Classeurs ignorés (présents dans CSV mais création non demandée)
        if result.get("classeurs_inconnus"):
            ctk.CTkLabel(
                inner,
                text=("⚠ Classeurs présents dans le CSV mais absents en local : "
                      f"{', '.join(result['classeurs_inconnus'])}"),
                font=("Outfit", 10),
                text_color=C["warning_text"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", pady=(4, 0))

        self._set_import_status("", C["text2"])

        # ── Détection des artworks alt non taggés ────────────────────────────
        # Les lignes CSV avec un artwork alternatif (N° Artwork > 0) qui ne
        # matchent pas le classeur tombent dans la catégorie
        # `artwork_alt_non_tagge` du diagnostic. On ouvre alors un dialog
        # dédié qui propose à l'utilisateur de choisir l'artwork
        # correspondant à sa carte physique parmi ceux connus de cardinfo.db.
        # Ce dialog est traité par module.import_csv.artwork_alt_resolver
        # + module.import_csv.artwork_alt_ui — le module anomalie reste
        # intouché.
        nt_artwork_alt = [
            nt for nt in (result.get("non_trouvees") or [])
            if nt.get("categorie") == "artwork_alt_non_tagge"
        ]
        # Note importante : `non_trouvees` est tronqué à
        # NON_TROUVEES_PREVIEW_MAX (50) côté importer_csv. Sur un CSV qui
        # déborderait largement (rare en pratique : il faudrait > 50
        # artworks alt non taggés), seules les 50 premières seront proposées.
        # C'est cohérent avec le reste du résumé d'import.
        if nt_artwork_alt:
            self._safe_after(150, self._ouvrir_dialog_artwork_alt, nt_artwork_alt)

        # Callback de rafraîchissement
        if self._on_update_cb and result["importees"] > 0:
            try:
                self._on_update_cb()
            except Exception:
                pass

    # ═══════════════════════════════════════════════════════════════════════
    # Résolution des artworks alt (nouveau)
    # ═══════════════════════════════════════════════════════════════════════

    def _ouvrir_dialog_artwork_alt(self, non_trouvees_artwork: list[dict]):
        """Ouvre le dialog de confirmation des artworks alternatifs.

        Étapes :
          1. Lookup cardinfo.db pour lister les artworks proposables par carte.
          2. Si propositions vides → pas de dialog (cas dégradé : cardinfo.db
             absente, ou aucun artwork alt finalement disponible).
          3. Sinon, ouverture du dialog modal. Le callback de validation
             applique les choix puis ajoute les classeurs touchés à
             FileAttenteClasseur (même mécanisme que création/anomalies).
        """
        if self._destroyed:
            return
        try:
            from module.import_csv.artwork_alt_resolver import (
                lister_propositions_artwork_alt,
            )
            propositions = lister_propositions_artwork_alt(non_trouvees_artwork)
        except Exception as e:
            self._afficher_resume_artwork_alt_erreur(str(e))
            return

        if not propositions:
            # Aucune proposition à montrer (cardinfo.db incomplète, ou
            # tous les artworks alt sont en fait déjà importés). On note
            # discrètement dans le résumé.
            self._afficher_resume_artwork_alt_aucune()
            return

        try:
            from module.import_csv.artwork_alt_ui import (
                afficher_dialog_artwork_alt,
            )
            afficher_dialog_artwork_alt(
                self,
                propositions=propositions,
                on_validate=self._on_artwork_alt_validate,
                on_close=None,
            )
        except Exception as e:
            self._afficher_resume_artwork_alt_erreur(str(e))

    def _on_artwork_alt_validate(self, decisions: list[dict]):
        """Callback appelé quand l'utilisateur confirme ses choix d'artwork."""
        if self._destroyed:
            return
        if not decisions:
            self._afficher_resume_artwork_alt_aucun_choix()
            return
        try:
            from module.import_csv.artwork_alt_resolver import (
                appliquer_choix_artworks,
            )
            res = appliquer_choix_artworks(decisions)
        except Exception as e:
            self._afficher_resume_artwork_alt_erreur(str(e))
            return

        # Déclenche le DL des nouvelles images via FileAttenteClasseur
        # (même pattern que dialog_anomalies._post_correction).
        # Le flag from_missing_check=True évite que le worker re-trigger un
        # rechargement complet du classeur côté UI parente.
        classeurs_touches = res.get("classeurs_touches", [])
        if classeurs_touches:
            try:
                from module.gestion_img.cache_images import clear_cache
                clear_cache()
            except Exception:
                pass
            try:
                from module.img_dl.file_attente_classeur import FileAttenteClasseur
                file = FileAttenteClasseur()
                for code in classeurs_touches:
                    tache = file.ajouter(code)
                    tache.from_missing_check = True
            except Exception as e:
                log.warning(f"_on_artwork_alt_validate FileAttenteClasseur : {e}")

        # Mise à jour du résumé d'import avec le résultat du dialog
        self._afficher_resume_artwork_alt_succes(res)

        # Callback de rafraîchissement parent (visualiseur de classeur etc.)
        if self._on_update_cb and res.get("appliquees", 0) > 0:
            try:
                self._on_update_cb()
            except Exception:
                pass

    def _afficher_resume_artwork_alt_succes(self, res: dict):
        """Ajoute une ligne au résumé d'import avec le bilan du dialog."""
        if self._destroyed:
            return
        try:
            n = res.get("appliquees", 0)
            n_echec = len(res.get("echecs", []))
            classeurs = res.get("classeurs_touches", [])

            text = f"🎨 Artworks alt confirmés : {n} ligne(s) importée(s)"
            if n_echec:
                text += f" ({n_echec} échec(s))"
            if classeurs:
                text += f"\n   Classeurs : {', '.join(classeurs)}"
                text += "\n   Téléchargement des images en cours en arrière-plan…"

            ctk.CTkLabel(
                self._import_result_frame,
                text=text,
                font=("Outfit", 10),
                text_color=C["gold"] if n > 0 else C["text3"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", padx=16, pady=(4, 8))
        except Exception:
            pass

    def _afficher_resume_artwork_alt_aucune(self):
        if self._destroyed:
            return
        try:
            ctk.CTkLabel(
                self._import_result_frame,
                text=("🎨 Artworks alt : aucune proposition disponible "
                      "(cardinfo.db incomplète ou non scannée)."),
                font=("Outfit", 10),
                text_color=C["text3"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", padx=16, pady=(4, 8))
        except Exception:
            pass

    def _afficher_resume_artwork_alt_aucun_choix(self):
        if self._destroyed:
            return
        try:
            ctk.CTkLabel(
                self._import_result_frame,
                text="🎨 Artworks alt : aucune sélection validée par l'utilisateur.",
                font=("Outfit", 10),
                text_color=C["text3"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", padx=16, pady=(4, 8))
        except Exception:
            pass

    def _afficher_resume_artwork_alt_erreur(self, message: str):
        if self._destroyed:
            return
        try:
            ctk.CTkLabel(
                self._import_result_frame,
                text=f"🎨 Artworks alt : erreur — {message[:120]}",
                font=("Outfit", 10),
                text_color=C["danger"],
                wraplength=W_WINDOW - 80,
                justify="left",
            ).pack(anchor="w", padx=16, pady=(4, 8))
        except Exception:
            pass

    def _on_import_error(self, exc: Exception):
        self._operation_en_cours = False
        self._reacquerir_grab_post_operation()
        self._set_import_status(
            f"❌ Erreur d'import : {exc}",
            C["danger"],
        )

    def _set_import_status(self, text: str, color: str):
        if self._destroyed:
            return
        try:
            self._import_status.configure(text=text, text_color=color)
        except Exception:
            pass

    # ═══════════════════════════════════════════════════════════════════════
    # Cycle de vie
    # ═══════════════════════════════════════════════════════════════════════

    def _on_close(self):
        if self._operation_en_cours:
            return  # ne pas fermer pendant export/import
        self._destroyed = True
        try:
            self.grab_release()
        except Exception:
            pass
        try:
            self.destroy()
        except Exception:
            pass

    # ── Gestion du grab pendant les opérations longues ──────────────────────
    # Le dialog est modal (grab_set au constructeur) pour empêcher l'utilisateur
    # de lancer plusieurs opérations en parallèle ou de naviguer pendant la
    # configuration. Mais une fois le worker thread démarré, on RELÂCHE le
    # grab pour que l'utilisateur puisse interagir avec le centre d'activité
    # et le reste de l'app pendant que l'import/export tourne en arrière-plan.
    # Le dialog reste visible mais ses boutons sont désactivés via
    # `_operation_en_cours` (cf. _action_import / _action_export).
    # Au retour (succès ou erreur) on RÉ-ACQUIERT le grab et on remet la
    # fenêtre au premier plan pour que l'utilisateur voie le résultat.

    def _release_grab_pendant_operation(self):
        """Libère le grab modal pour la durée de l'opération en cours."""
        try:
            self.grab_release()
        except Exception:
            pass

    def _reacquerir_grab_post_operation(self):
        """Ré-acquiert le grab modal et ramène la fenêtre au premier plan."""
        if self._destroyed:
            return
        try:
            self.lift()
        except Exception:
            pass
        try:
            self.focus_force()
        except Exception:
            pass
        try:
            self.grab_set()
        except Exception:
            pass

    def _safe_after(self, delay, fn, *args):
        if self._destroyed:
            return
        try:
            self.after(delay, fn, *args)
        except Exception:
            pass


def show_import_export_dialog(parent,
                              classeur_initial: str | None = None,
                              onglet_initial: str = "export",
                              on_update=None):
    """
    Helper public : ouvre la fenêtre Import/Export et la rend modale.

    Args:
        parent           : widget parent.
        classeur_initial : code de classeur à pré-sélectionner.
        onglet_initial   : "export" ou "import".
        on_update        : callback après un import réussi.
    """
    win = DialogImportExport(
        parent,
        classeur_initial=classeur_initial,
        onglet_initial=onglet_initial,
        on_update=on_update,
    )
    win.focus_force()
    return win
