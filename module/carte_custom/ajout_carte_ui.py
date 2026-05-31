"""
ajout_carte_ui.py — Dialogue "Ajouter une carte" (clic droit classeur).

Flux :
  1. Recherche par nom (cardinfo.db) → liste de cartes
  2. Sélection d'une carte → affichage de ses artworks connus
  3. Choix d'un artwork (ou « Aucun / artwork manquant »)
  4. set_code cible + rareté (autocomplétion depuis le référentiel)
  5. Ajout → is_custom=1, puis callback de refresh

Réutilise le service module.carte_custom.ajout_carte_service.
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
import customtkinter as ctk
from concurrent.futures import ThreadPoolExecutor
from PIL import Image

from module.theme import C
from module.ui.composants import gold_button, secondary_button
from module.logger_app import log
from module.centralisation_dossier import IMAGES_SMALL_FOLDER
from module.carte_custom.ajout_carte_service import (
    rechercher_cartes_par_nom, get_artworks_carte, resoudre_set_name,
    ajouter_carte_au_classeur,
)
from module.gestion_rarete.raretes_reference import lister_raretes_reference, name_to_code

# Dimensions des vignettes d'artwork dans la liste de sélection
_ART_W, _ART_H = 70, 102


class AjoutCarteDialog(ctk.CTkToplevel):
    """Dialogue modal d'ajout manuel d'une carte à un classeur."""

    def __init__(self, parent, classeur_code: str,
                 set_prefix: str = "", on_done=None):
        super().__init__(parent)
        self.title("Ajouter une carte au classeur")
        self.geometry("720x640")
        self.configure(fg_color=C["bg2"])
        self.transient(parent)

        self._classeur = classeur_code
        self._set_prefix = (set_prefix or classeur_code or "").strip().upper()
        self._on_done = on_done
        self._cartes: list[dict] = []
        self._carte_sel: dict | None = None
        self._artworks: list[dict] = []
        self._art_var = ctk.StringVar(value="-1")  # index artwork ; -1 = aucun

        # Infra previews async (téléchargement vignettes d'artwork)
        self._preview_cache: dict = {}
        self._preview_executor = ThreadPoolExecutor(max_workers=4)
        self._destroyed = False
        self.protocol("WM_DELETE_WINDOW", self._on_close)

        self._build()
        # Le trace n'est posé QU'APRÈS _build() : ainsi _on_search ne peut
        # jamais s'exécuter avant que _results / _art_frame existent.
        self._search_var.trace_add("write", lambda *_: self._on_search())
        self.after(120, self._focus_search)

    # ── Construction UI ──────────────────────────────────────────────────
    def _build(self):
        ctk.CTkLabel(
            self, text="Ajouter une carte",
            font=("Outfit", 20, "bold"), text_color=C["gold"],
        ).pack(anchor="w", padx=20, pady=(18, 2))
        ctk.CTkLabel(
            self,
            text=("Recherche la carte par son nom (catalogue), choisis un "
                  "artwork, puis renseigne le code et la rareté pour CE set. "
                  "La carte sera marquée comme ajout manuel."),
            font=("Outfit", 11), text_color=C["text"], wraplength=660,
            justify="left",
        ).pack(anchor="w", padx=20, pady=(0, 12))

        # 1) Recherche par nom
        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(fill="x", padx=20)
        self._search_var = ctk.StringVar()
        self._search = ctk.CTkEntry(
            row, textvariable=self._search_var,
            placeholder_text="Nom de la carte (ex : Blue-Eyes White Dragon)",
            height=36,
        )
        self._search.pack(fill="x")

        # Liste résultats (scrollable)
        self._results = ctk.CTkScrollableFrame(self, fg_color=C["bg3"], height=150)
        self._results.pack(fill="x", padx=20, pady=(8, 4))

        # 2) Artworks de la carte sélectionnée
        ctk.CTkLabel(
            self, text="Artwork", font=("Outfit", 13, "bold"),
            text_color=C["text"],
        ).pack(anchor="w", padx=20, pady=(8, 0))
        self._art_frame = ctk.CTkScrollableFrame(self, fg_color=C["bg3"], height=220)
        self._art_frame.pack(fill="x", padx=20, pady=(4, 4))
        self._render_artworks()  # état initial (vide)

        # 3) Champs cible : set_code + rareté
        grid = ctk.CTkFrame(self, fg_color="transparent")
        grid.pack(fill="x", padx=20, pady=(8, 4))

        ctk.CTkLabel(grid, text="Code complet (set_code)", font=("Outfit", 11),
                     text_color=C["text"]).grid(row=0, column=0, sticky="w")
        ctk.CTkLabel(grid, text="Rareté", font=("Outfit", 11),
                     text_color=C["text"]).grid(row=0, column=1, sticky="w", padx=(12, 0))

        # Pré-remplissage : si le préfixe contient déjà un code langue
        # (ex 'LOCR-JP' pour un set OCG), on ne ré-ajoute pas '-EN'. Sinon
        # on suggère '-EN'. Dans tous les cas l'utilisateur DOIT compléter
        # avec le numéro (ex 005) → le placeholder le rappelle.
        import re as _re
        pref = self._set_prefix
        if pref and _re.search(r"-[A-Z]{2}$", pref):
            valeur_init = pref          # ex 'LOCR-JP' → garder tel quel
            exemple = f"{pref}001"
        elif pref:
            valeur_init = f"{pref}-EN"
            exemple = f"{pref}-EN001"
        else:
            valeur_init = ""
            exemple = "RA05-EN015"
        self._code_var = ctk.StringVar(value=valeur_init)
        ctk.CTkEntry(grid, textvariable=self._code_var, width=240,
                     placeholder_text=f"ex : {exemple}").grid(
            row=1, column=0, sticky="w", pady=(2, 0))

        self._rarete_var = ctk.StringVar()
        self._rarete_box = ctk.CTkComboBox(
            grid, variable=self._rarete_var, width=300,
            values=lister_raretes_reference(),
        )
        self._rarete_box.set("")
        self._rarete_box.grid(row=1, column=1, sticky="w", padx=(12, 0), pady=(2, 0))

        # Aide explicite sous le champ code : le numéro est OBLIGATOIRE.
        ctk.CTkLabel(
            grid,
            text=(f"⚠  Ajoute le numéro de la carte dans le set "
                  f"(ex : {exemple}). Sans numéro, la carte sera mal classée."),
            font=("Outfit", 10), text_color=C["gold"],
            wraplength=560, justify="left",
        ).grid(row=2, column=0, columnspan=2, sticky="w", pady=(4, 0))

        # Message d'état
        self._msg = ctk.CTkLabel(self, text="", font=("Outfit", 11),
                                 text_color=C["text"], wraplength=660,
                                 justify="left")
        self._msg.pack(anchor="w", padx=20, pady=(6, 0))

        # Boutons
        btns = ctk.CTkFrame(self, fg_color="transparent")
        btns.pack(fill="x", padx=20, pady=14, side="bottom")
        gold_button(btns, "Ajouter la carte", command=self._on_add).pack(side="right")
        secondary_button(btns, "Fermer", command=self.destroy).pack(side="right", padx=(0, 8))

    def _focus_search(self):
        try:
            self._search.focus_set()
        except Exception:
            pass

    # ── Recherche ────────────────────────────────────────────────────────
    def _on_search(self):
        if not hasattr(self, "_results"):
            return  # _build pas terminé : rien à faire
        terme = self._search_var.get().strip()
        for w in self._results.winfo_children():
            w.destroy()
        if len(terme) < 2:
            self._cartes = []
            return
        self._cartes = rechercher_cartes_par_nom(terme, limit=30)
        if not self._cartes:
            ctk.CTkLabel(self._results, text="Aucune carte trouvée.",
                         font=("Outfit", 11), text_color=C["text"]).pack(
                anchor="w", padx=8, pady=6)
            return
        for c in self._cartes:
            label = c["name_en"] or c["name_fr"]
            if c["name_fr"] and c["name_fr"] != c["name_en"]:
                label += f"   ·   {c['name_fr']}"
            b = ctk.CTkButton(
                self._results, text=label, anchor="w", height=30,
                fg_color="transparent", hover_color=C["bg2"],
                text_color=C["text"],
                command=lambda cc=c: self._select_carte(cc),
            )
            b.pack(fill="x", padx=2, pady=1)

    def _select_carte(self, carte: dict):
        self._carte_sel = carte
        self._search_var.set(carte["name_en"] or carte["name_fr"])
        self._artworks = get_artworks_carte(carte["card_uuid"])
        self._art_var.set("-1")
        self._render_artworks()
        self._set_msg(f"Carte sélectionnée : {carte['name_en'] or carte['name_fr']} "
                      f"— {len(self._artworks)} artwork(s) connu(s).")

    # ── Artworks ─────────────────────────────────────────────────────────
    def _render_artworks(self):
        for w in self._art_frame.winfo_children():
            w.destroy()

        # Option « aucun » toujours disponible (artwork manquant / placeholder)
        none_row = ctk.CTkFrame(self._art_frame, fg_color="transparent")
        none_row.pack(fill="x", padx=4, pady=3)
        ctk.CTkRadioButton(
            none_row, text="Aucun (artwork manquant — placeholder)",
            variable=self._art_var, value="-1",
            font=("Outfit", 11), text_color=C["text"],
        ).pack(side="left", padx=(4, 0))

        for i, art in enumerate(self._artworks):
            row = ctk.CTkFrame(self._art_frame, fg_color=C["bg2"], corner_radius=6)
            row.pack(fill="x", padx=4, pady=3)

            radio = ctk.CTkRadioButton(
                row, text="", variable=self._art_var, value=str(i),
                width=24,
            )
            radio.pack(side="left", padx=(8, 6), pady=6)

            # Vignette (placeholder le temps du DL)
            thumb = ctk.CTkLabel(
                row, text="…", width=_ART_W, height=_ART_H,
                fg_color=C["bg3"], corner_radius=4,
                text_color=C["text"],
            )
            thumb.pack(side="left", padx=(0, 10), pady=6)

            aid = art.get("card_image_id")
            txt = f"Artwork #{i + 1}" + (f"   ·   id {aid}" if aid is not None else "")
            lbl = ctk.CTkLabel(row, text=txt, font=("Outfit", 11),
                               text_color=C["text"], anchor="w")
            lbl.pack(side="left", fill="x", expand=True)

            # Clic n'importe où sur la ligne → sélectionne ce radio
            def _pick(idx=i):
                self._art_var.set(str(idx))
            for w in (row, thumb, lbl):
                w.bind("<Button-1>", lambda _e, f=_pick: f())

            # Téléchargement async de la vignette
            self._charger_preview_async(
                aid,
                art.get("card_image_small") or art.get("card_image_url"),
                art.get("card_image_url"),
                thumb,
            )

    # ── Previews async (vignettes d'artwork) ─────────────────────────────
    def _charger_preview_async(self, image_id, url_primary, url_fallback, label):
        """Charge la vignette : cache → disque → téléchargement async."""
        if not image_id:
            try:
                if label.winfo_exists():
                    label.configure(text="—")
            except Exception:
                pass
            return

        cached = self._preview_cache.get(image_id)
        if cached is not None:
            self._afficher_image(image_id, None, label, cached)
            return

        path = os.path.join(IMAGES_SMALL_FOLDER, f"{image_id}.jpg")
        if os.path.exists(path):
            self._afficher_image(image_id, path, label, None)
            return

        try:
            from module.config_image_source import build_image_url
            url_eff = build_image_url(url_primary, image_id) or url_primary
        except Exception:
            url_eff = url_primary
        url_eff = url_eff or url_fallback
        if not url_eff:
            try:
                if label.winfo_exists():
                    label.configure(text="?")
            except Exception:
                pass
            return

        def worker():
            try:
                from module.img_dl.telechargement_service import TelechargementService
                os.makedirs(IMAGES_SMALL_FOLDER, exist_ok=True)
                service = TelechargementService()
                try:
                    service.telecharger_image(url_eff, path)
                except Exception:
                    pass
                if (not os.path.exists(path) and url_fallback
                        and url_fallback != url_eff):
                    try:
                        service.telecharger_image(url_fallback, path)
                    except Exception:
                        pass
                if self._destroyed:
                    return
                if os.path.exists(path):
                    self.after(0, self._afficher_image, image_id, path, label, None)
            except Exception:
                pass

        try:
            self._preview_executor.submit(worker)
        except Exception:
            pass

    def _afficher_image(self, image_id, path, label, ctk_img):
        if self._destroyed:
            return
        try:
            if not label.winfo_exists():
                return
        except Exception:
            return
        try:
            img = ctk_img
            if img is None and path:
                pil = Image.open(path).convert("RGB").resize(
                    (_ART_W, _ART_H), Image.LANCZOS)
                img = ctk.CTkImage(pil, size=(_ART_W, _ART_H))
                self._preview_cache[image_id] = img
            if img is not None:
                label.configure(image=img, text="")
                setattr(label, "_img_ref", img)
        except Exception:
            try:
                label.configure(text="×")
            except Exception:
                pass

    def _on_close(self):
        self._destroyed = True
        try:
            self._preview_executor.shutdown(wait=False)
        except Exception:
            pass
        self.destroy()

    def _selected_artwork(self) -> dict | None:
        try:
            idx = int(self._art_var.get())
        except (TypeError, ValueError):
            idx = -1
        if idx < 0 or idx >= len(self._artworks):
            return None
        return self._artworks[idx]

    # ── Ajout ────────────────────────────────────────────────────────────
    def _on_add(self):
        set_code = self._code_var.get().strip().upper()
        rarete = self._rarete_var.get().strip()
        if not self._carte_sel:
            self._set_msg("Sélectionne d'abord une carte dans la liste.", err=True)
            return
        if not set_code:
            self._set_msg("Renseigne le code complet de la carte (set_code).", err=True)
            return
        # Le code DOIT se terminer par un numéro (ex LOCR-JP001). Sans numéro,
        # le tri placerait la carte n'importe où. On exige au moins un chiffre
        # en fin de code, après un code langue.
        import re as _re
        if not _re.search(r"\d+$", set_code):
            self._set_msg(
                "Le code doit inclure le NUMÉRO de la carte dans le set "
                "(ex : LOCR-JP001). Complète le code avant d'ajouter.",
                err=True)
            return
        if not rarete:
            self._set_msg("Renseigne la rareté.", err=True)
            return

        # Garde-fou : une rareté NON reconnue serait enregistrée silencieusement
        # comme « rareté à venir » et stockée sur la carte. C'est ainsi que des
        # libellés erronés (ex : « New artwork ») se retrouvaient affichés comme
        # raretés. On confirme explicitement avant de créer une nouvelle rareté.
        # name_to_code() reflète exactement la logique de normaliser_rarete()
        # SANS effet de bord (ne crée rien) : None ⇒ inconnue.
        if name_to_code(rarete) is None:
            from tkinter import messagebox
            creer = messagebox.askyesno(
                "Rareté inconnue",
                f"« {rarete} » n'est pas une rareté connue.\n\n"
                f"La créer comme nouvelle rareté ?\n\n"
                f"Choisis « Non » si tu voulais sélectionner une rareté "
                f"existante dans la liste déroulante.",
                parent=self,
            )
            if not creer:
                self._set_msg(
                    "Ajout annulé : sélectionne une rareté dans la liste.",
                    err=True,
                )
                return

        ok, msg = ajouter_carte_au_classeur(
            classeur=self._classeur,
            set_code=set_code,
            rarete_saisie=rarete,
            card_uuid=self._carte_sel["card_uuid"],
            artwork=self._selected_artwork(),
        )
        self._set_msg(msg, err=not ok)
        if ok:
            # Rafraîchir la liste de raretés (une « à venir » a pu être créée)
            self._rarete_box.configure(values=lister_raretes_reference())
            if self._on_done:
                try:
                    self._on_done()
                except Exception as e:
                    log.warning(f"AjoutCarteDialog on_done : {e}")

    def _set_msg(self, text: str, err: bool = False):
        self._msg.configure(text=text,
                            text_color=(C["danger_text"] if err else C["gold"]))


def ouvrir_dialog_ajout_carte(parent, classeur_code: str,
                              set_prefix: str = "", on_done=None):
    """Helper d'ouverture du dialogue d'ajout de carte."""
    dlg = AjoutCarteDialog(parent, classeur_code, set_prefix, on_done)
    dlg.grab_set()
    return dlg
