"""
ecran_selecteur_set.py — Écran 2 : sélection d'un set.

Cycle 5 — Refonte UI pour performance et UX :
  1. SOURCE DE DONNÉES : lit cardinfo.db en priorité (instantané, offline),
     fallback sur API YGOPRODeck uniquement si BDD absente/incomplète OU si
     l'utilisateur clique sur "↻ Sync" (force_api=True).

  2. ÉCRAN VIDE PAR DÉFAUT : aucune carte affichée à l'ouverture. L'utilisateur
     doit soit taper au moins 2 caractères dans le champ texte, soit choisir
     dans la liste déroulante complète. Cela élimine le coût de construction
     de 80 SetCard à chaque ouverture + filtrage.

  3. COMBOBOX COMPLÈTE : liste déroulante native OS — aucun widget SetCard
     créé tant qu'on ne sélectionne rien. Permet aux utilisateurs qui ne
     connaissent pas les codes d'explorer sans générer de widgets.

  4. DEBOUNCE 150 ms + PRÉ-INDEX : filtrage du champ texte ne se déclenche
     qu'après pause de saisie. Comparaisons effectuées sur code/nom déjà
     upper-cased une seule fois au _on_loaded (pas à chaque frappe).

  5. CAP 50 RÉSULTATS : évite de construire >80 SetCard pour une recherche
     trop large. Message "Affinez votre recherche…" si plus de 50 matches.
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

import threading
import customtkinter as ctk

from module.theme import C
from module.ui.composants import (
    gold_button, icon_button,
    Navbar, separator, search_entry, styled_combobox,
    CentreActiviteButton,
)
from module.creation_classeur.creation_classeur_service import get_available_set_codes
from module.img_dl.file_attente_classeur import FileAttenteClasseur
from module.config_langue import load_langue

_file_attente = FileAttenteClasseur()

# Seuil minimum de caractères pour déclencher une recherche texte
MIN_SEARCH_CHARS = 2
# Cap de résultats affichés pour éviter de créer trop de widgets
MAX_RESULTS = 50
# Délai après la dernière frappe avant de lancer le filtrage (ms)
SEARCH_DEBOUNCE_MS = 150


class SetCard(ctk.CTkFrame):
    def __init__(self, parent, code, nom, count, tcg_date="", on_click=None):
        super().__init__(parent, fg_color=C["bg_card"], border_color=C["border"],
                         border_width=1, corner_radius=8, cursor="hand2")
        self._nom_lbl = None
        self._build(code, nom, count, tcg_date, on_click)

    def _build(self, code, nom, count, tcg_date, on_click):
        top = ctk.CTkFrame(self, fg_color="transparent")
        top.pack(fill="x", padx=12, pady=(10, 4))

        ctk.CTkLabel(top, text=code,
                     fg_color=C["bg3"], text_color=C["gold"],
                     font=("Consolas", 9), corner_radius=4,
                     padx=6, pady=2).pack(side="left")
        if tcg_date:
            ctk.CTkLabel(top, text=f"📅 {tcg_date[:7]}",
                         font=("Segoe UI", 8), text_color=C["text3"]).pack(side="right")

        self._nom_lbl = ctk.CTkLabel(self, text=nom, font=("Outfit", 11, "bold"),
                                      text_color=C["text"], anchor="w",
                                      wraplength=170, justify="left")
        self._nom_lbl.pack(anchor="w", padx=12, pady=(0, 4))
        ctk.CTkLabel(self, text=f"# {count} cartes", font=("Consolas", 9),
                     text_color=C["text3"]).pack(anchor="w", padx=12, pady=(0, 10))

        def enter(e):
            self.configure(border_color=C["gold_dim"])
            self._nom_lbl.configure(text_color=C["gold_hover"])
        def leave(e):
            self.configure(border_color=C["border"])
            self._nom_lbl.configure(text_color=C["text"])
        def click(e):
            if on_click:
                on_click(code, nom)

        for w in _all(self):
            w.bind("<Enter>", enter)
            w.bind("<Leave>", leave)
            w.bind("<Button-1>", click)
        self.bind("<Enter>", enter)
        self.bind("<Leave>", leave)
        self.bind("<Button-1>", click)


def _all(widget):
    yield widget
    for c in widget.winfo_children():
        yield from _all(c)


class OverlayCreation(ctk.CTkToplevel):
    def __init__(self, parent, code):
        super().__init__(parent)
        self.title("")
        self.geometry("320x160")
        self.resizable(False, False)
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()
        ctk.CTkLabel(self, text="⏳", font=("Segoe UI", 32),
                     text_color=C["gold"]).pack(pady=(24, 4))
        ctk.CTkLabel(self, text=f"Création du classeur {code}…",
                     font=("Outfit", 11, "bold"), text_color=C["text"]).pack()
        ctk.CTkLabel(self, text="Récupération depuis YGOPRODeck…",
                     font=("Outfit", 10), text_color=C["text3"]).pack(pady=4)

    def close(self):
        try:
            self.grab_release()
            self.destroy()
        except Exception:
            pass


class EcranSelecteurSet(ctk.CTkFrame):
    """Écran de sélection d'un set.

    Attributs clés :
        _all_sets       : liste brute [(code, "CODE (nom) [nb]"), ...]
        _searchable     : liste pré-indexée [(code_up, nom_up, nb, code_raw, nom_raw), ...]
        _search_after   : ID du timer de debounce en cours (ou None)
        _combo_var      : StringVar liée au CTkComboBox
        _search_var     : StringVar liée au champ texte
    """

    def __init__(self, parent, navigate_to=None):
        super().__init__(parent, fg_color=C["bg"], corner_radius=0)
        self._navigate_to = navigate_to
        self._all_sets:    list[tuple[str, str]]                = []
        self._searchable:  list[tuple[str, str, int, str, str]] = []
        self._search_after = None
        self._build()

    # ── Construction UI ──────────────────────────────────────────────────

    def _build(self):
        Navbar(self, title="Choisir un Set",
               show_back=True, back_command=self._retour,
               right_factory=self._build_nav_right).pack(fill="x")

        # Zone contrôles (2 lignes : combobox + champ texte)
        controls = ctk.CTkFrame(self, fg_color="transparent")
        controls.pack(fill="x", padx=24, pady=(16, 0))
        controls.columnconfigure(0, weight=1)

        # Ligne 1 : champ texte + bouton Sync
        self._search_var = ctk.StringVar()
        search_entry(
            controls, textvariable=self._search_var,
            placeholder=f"🔍  Rechercher par nom ou code "
                         f"(min. {MIN_SEARCH_CHARS} caractères)…",
        ).grid(row=0, column=0, sticky="ew", padx=(0, 12))
        self._search_var.trace_add("write", self._on_search_change)

        icon_button(controls, "↻ Sync",
                    command=self._synchroniser_api).grid(row=0, column=1)

        # Ligne 2 : combobox liste complète
        self._combo_var = ctk.StringVar(value="")
        self._combo = styled_combobox(
            controls,
            values=[],
            variable=self._combo_var,
            width=400,
        )
        self._combo.grid(row=1, column=0, columnspan=2,
                         sticky="ew", pady=(10, 0))
        self._combo.set("")
        # configure() pour la callback "après sélection" :
        self._combo.configure(command=self._on_combo_change)
        self._update_combo_placeholder("Chargement de la liste…")

        # Labels de statut
        self._lbl_count = ctk.CTkLabel(self, text="Chargement…",
                                        font=("Outfit", 10), text_color=C["text3"])
        self._lbl_count.pack(anchor="w", padx=26, pady=(10, 0))

        self._lbl_err = ctk.CTkLabel(self, text="", font=("Outfit", 10),
                                      text_color=C["danger_text"])
        self._lbl_err.pack(anchor="w", padx=26)

        separator(self).pack(fill="x", padx=24, pady=(4, 12))

        # Zone scrollable (cartes ou état vide)
        self._scroll = ctk.CTkScrollableFrame(self, fg_color="transparent")
        self._scroll.pack(fill="both", expand=True, padx=16)

        # État initial : message d'invite (aucun widget lourd créé)
        self._render_prompt()

    def _build_nav_right(self, parent):
        """Boutons d'action à droite de la navbar.

        Le bouton "← Retour" est désormais affiché à GAUCHE par le composant
        Navbar standard (show_back=True dans _build), comme dans EcranOptions
        et EcranClasseur — cohérence UI sur tous les écrans secondaires.
        """
        # 📥 Activité — visible aussi ici car la création depuis ce
        # sélecteur déclenche immédiatement une tâche dans la file.
        CentreActiviteButton(parent).pack(side="left", padx=4)

    # ── Placeholder combobox ─────────────────────────────────────────────

    def _update_combo_placeholder(self, text: str):
        """Met un texte d'invite dans la combobox (avant chargement)."""
        try:
            self._combo.configure(values=[text])
            # On ne set pas la var pour ne pas déclencher la callback ;
            # l'utilisateur verra le placeholder en ouvrant le dropdown.
        except Exception:
            pass

    # ── Chargement initial / synchronisation ─────────────────────────────

    def charger(self):
        """Appelé par NavigationController après navigation vers ce screen.

        Si les sets n'ont pas encore été chargés, lance la synchronisation.
        """
        if not self._all_sets:
            self._charger_sets(force_api=False)

    def _synchroniser_api(self):
        """Bouton "↻ Sync" : force la relecture depuis l'API YGOPRODeck."""
        self._charger_sets(force_api=True)

    def _charger_sets(self, force_api: bool):
        self._lbl_err.configure(text="")
        self._lbl_count.configure(
            text=("Chargement depuis YGOPRODeck (Sync)…" if force_api
                  else "Chargement de la liste des sets…")
        )
        self._update_combo_placeholder("Chargement de la liste…")
        # Reset UI cartes pendant le chargement
        for w in self._scroll.winfo_children():
            w.destroy()
        threading.Thread(
            target=self._fetch, args=(force_api,), daemon=True
        ).start()

    def _fetch(self, force_api: bool):
        try:
            use_fr = load_langue() == "FR"
            sets = get_available_set_codes(use_fr=use_fr, force_api=force_api)
            self.after(0, self._on_loaded, sets, None)
        except Exception as e:
            self.after(0, self._on_loaded, [], str(e))

    def _on_loaded(self, sets, error):
        if error:
            self._lbl_err.configure(text=f"⚠ Erreur : {error}")
            self._lbl_count.configure(text="Impossible de charger les sets.")
            self._update_combo_placeholder("⚠ Erreur de chargement")
            return

        self._all_sets = sets

        # Pré-indexation : upper-case UNE SEULE FOIS (évite N upper() par frappe)
        self._searchable = []
        for code, label in sets:
            # label format : "CODE (Nom) [nb]"
            try:
                nom_raw = label.split(" (", 1)[1].rsplit(") [", 1)[0]
            except (IndexError, ValueError):
                nom_raw = code
            try:
                nb = int(label.rsplit("[", 1)[1].split("]", 1)[0])
            except (ValueError, IndexError):
                nb = 0
            self._searchable.append(
                (code.upper(), nom_raw.upper(), nb, code, nom_raw)
            )

        # Peuple la combobox avec tous les sets (format "CODE — Nom")
        combo_values = [f"{c} — {n}" for _, _, _, c, n in self._searchable]
        try:
            self._combo.configure(values=combo_values)
            self._combo.set("")
        except Exception:
            pass

        nb = len(sets)
        self._lbl_count.configure(
            text=f"{nb:,} set{'s' if nb > 1 else ''} disponible"
                 f"{'s' if nb > 1 else ''}  "
                 f"·  Tapez {MIN_SEARCH_CHARS}+ caractères ou choisissez dans la liste"
        )

        # Réapplique les filtres si l'utilisateur a déjà commencé à taper
        # pendant le chargement (cas rare mais propre)
        self._filtrer()

    # ── Recherche texte avec debounce ────────────────────────────────────

    def _on_search_change(self, *_):
        """Appelé à chaque frappe — debounce pour éviter les rafales."""
        if self._search_after is not None:
            try:
                self.after_cancel(self._search_after)
            except Exception:
                pass
        self._search_after = self.after(SEARCH_DEBOUNCE_MS, self._filtrer)

    def _filtrer(self):
        self._search_after = None
        terme = self._search_var.get().strip().upper()

        # Pas encore chargé
        if not self._searchable:
            self._render_prompt()
            return

        # Sous le seuil minimum → état invite (pas de widgets construits)
        if len(terme) < MIN_SEARCH_CHARS:
            self._render_prompt(current_len=len(terme))
            return

        # Filtrage sur index pré-calculé : uppercase déjà fait une fois
        matches = [
            (code_raw, nom_raw, nb)
            for code_up, nom_up, nb, code_raw, nom_raw in self._searchable
            if terme in code_up or terme in nom_up
        ]

        self._render_results(matches, terme)

    # ── Sélection combobox ───────────────────────────────────────────────

    def _on_combo_change(self, choice: str):
        """Appelé quand l'utilisateur sélectionne un set dans la combobox.

        choice est du format "CODE — Nom". On extrait le code, affiche la
        carte correspondante dans le scroll pour confirmation visuelle, puis
        le clic utilisateur sur la carte déclenche la création.
        """
        if not choice or " — " not in choice:
            return
        code = choice.split(" — ", 1)[0].strip()
        # Trouver l'entrée correspondante dans _searchable
        for code_up, nom_up, nb, code_raw, nom_raw in self._searchable:
            if code_raw == code:
                self._render_results([(code_raw, nom_raw, nb)], code_raw.upper())
                break

    # ── Rendu : état invite (aucun widget) ───────────────────────────────

    def _render_prompt(self, current_len: int = 0):
        """Message d'invitation à la recherche — aucune SetCard créée."""
        for w in self._scroll.winfo_children():
            w.destroy()

        f = ctk.CTkFrame(self._scroll, fg_color="transparent")
        f.pack(expand=True, pady=60)

        ctk.CTkLabel(
            f, text="🔍", font=("Segoe UI", 48),
            text_color=C["bg_hover"],
        ).pack()

        if current_len == 0:
            title_txt = "Trouvez votre set"
            hint_txt  = (f"Tapez au moins {MIN_SEARCH_CHARS} caractères "
                         f"dans la barre de recherche,\n"
                         f"ou choisissez directement dans la liste déroulante.")
        else:
            remaining = MIN_SEARCH_CHARS - current_len
            title_txt = "Presque…"
            hint_txt  = (f"Encore {remaining} caractère"
                         f"{'s' if remaining > 1 else ''} pour lancer "
                         f"la recherche.")

        ctk.CTkLabel(
            f, text=title_txt,
            font=("Georgia", 16, "bold"), text_color=C["text2"],
        ).pack(pady=(12, 4))

        ctk.CTkLabel(
            f, text=hint_txt,
            font=("Outfit", 11), text_color=C["text3"],
            justify="center",
        ).pack()

    # ── Rendu : résultats ────────────────────────────────────────────────

    def _render_results(self, matches: list[tuple[str, str, int]], terme: str):
        """Affiche les cartes des sets qui matchent (cap à MAX_RESULTS)."""
        for w in self._scroll.winfo_children():
            w.destroy()

        nb_total = len(matches)

        if nb_total == 0:
            f = ctk.CTkFrame(self._scroll, fg_color="transparent")
            f.pack(expand=True, pady=60)
            ctk.CTkLabel(
                f, text="∅", font=("Segoe UI", 48),
                text_color=C["bg_hover"],
            ).pack()
            ctk.CTkLabel(
                f, text=f"Aucun set trouvé pour « {terme} »",
                font=("Outfit", 12, "bold"), text_color=C["text2"],
            ).pack(pady=(10, 4))
            ctk.CTkLabel(
                f, text="Vérifiez l'orthographe ou le code, "
                        "ou cliquez sur ↻ Sync pour rafraîchir la liste.",
                font=("Outfit", 10), text_color=C["text3"],
                justify="center",
            ).pack()
            return

        # Cap les résultats pour éviter de construire >MAX_RESULTS widgets
        capped = matches[:MAX_RESULTS]
        cols   = 4

        for i, (code, nom, count) in enumerate(capped):
            row, col = divmod(i, cols)
            SetCard(
                self._scroll, code, nom, count,
                on_click=self._creer,
            ).grid(row=row, column=col, padx=8, pady=8, sticky="nsew")

        for c in range(cols):
            self._scroll.columnconfigure(c, weight=1)

        # Message en bas si la recherche dépasse le cap
        if nb_total > MAX_RESULTS:
            rows_used = -(-len(capped) // cols)  # ceil
            hint = ctk.CTkLabel(
                self._scroll,
                text=f"⚠  Affichage limité à {MAX_RESULTS} résultats sur "
                     f"{nb_total}. Affinez votre recherche pour voir les "
                     f"autres.",
                font=("Outfit", 10, "italic"), text_color=C["gold_dim"],
                justify="center",
            )
            hint.grid(row=rows_used, column=0, columnspan=cols,
                      pady=(12, 0), sticky="ew")

    # ── Création d'un classeur ───────────────────────────────────────────

    def _creer(self, code, nom):
        overlay = OverlayCreation(self.winfo_toplevel(), code)
        def worker():
            try:
                _file_attente.ajouter(code, nom)
                self.after(500, overlay.close)
                self.after(600, lambda: self._navigate_to("classeur", code=code))
            except Exception as e:
                self.after(0, overlay.close)
                self.after(0, lambda: self._lbl_err.configure(
                    text=f"⚠ {e}"))
        threading.Thread(target=worker, daemon=True).start()

    def _retour(self):
        if self._navigate_to:
            self._navigate_to("accueil")
