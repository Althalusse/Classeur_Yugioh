"""
artwork_alt_ui.py — Dialog de confirmation des artworks alternatifs CSV.

Ouvert automatiquement à la fin d'un import CSV si des lignes ont été
classées `artwork_alt_non_tagge` par le diagnostic. Affiche pour chaque
carte concernée :
  - les artworks alt connus globalement (preview image)
  - les lignes CSV affectées (1 par rareté)
  - un choix d'artwork par ligne avec bouton "appliquer à toutes les raretés"

À la confirmation, délègue à artwork_alt_resolver.appliquer_choix_artworks()
qui INSERT les nouvelles lignes dans le classeur + UPDATE qty/qualité/édition,
puis le caller (dialog_import_export) déclenche FileAttenteClasseur pour
télécharger les images.

Patterns réutilisés depuis module.ui.dialog_anomalies :
  - téléchargement preview async via ThreadPoolExecutor (max 4 threads)
  - cache PIL → disque → DL pour les images
  - `widget.after(0, ...)` pour les updates UI thread-safe
  - destruction propre via flag `_destroyed`
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
from concurrent.futures import ThreadPoolExecutor

import customtkinter as ctk
from PIL import Image

from module.theme import C
from module.ui.composants import gold_button, secondary_button
from module.centralisation_dossier import IMAGES_SMALL_FOLDER
from module.logger_app import log


# Ratio carte YGO : 59:86
PREVIEW_W = 90
PREVIEW_H = int(PREVIEW_W * 86 / 59)   # ≈ 131 px

# "Aucun choix" — placeholder pour ligne CSV non confirmée
CHOIX_AUCUN = "__AUCUN__"


class ArtworkAltDialog(ctk.CTkToplevel):
    """
    Dialog modal de confirmation des artworks alternatifs.

    Args:
        parent       : widget parent (typiquement le dialog import/export).
        propositions : liste retournée par lister_propositions_artwork_alt().
                       Chaque entrée = 1 carte avec ses raretés CSV +
                       artworks alt proposés.
        on_validate  : callback appelé avec la liste de décisions
                       construite à partir des choix utilisateur.
                       Format : list[dict] cf appliquer_choix_artworks().
                       Si None, l'utilisateur ne pourra pas valider.
        on_close     : callback appelé au moment de la fermeture (validée
                       ou annulée). Reçoit (validated: bool).
    """

    def __init__(self, parent, propositions: list[dict],
                 on_validate=None, on_close=None,
                 title: str | None = None,
                 subtitle: str | None = None):
        super().__init__(parent)
        self.title(title or "Artworks alternatifs — confirmation")
        self.geometry("960x680")
        self.configure(fg_color=C["bg2"])
        try:
            self.attributes("-topmost", True)
        except Exception:
            pass

        self._propositions = propositions or []
        self._on_validate  = on_validate
        self._on_close     = on_close
        self._destroyed    = False
        self._validated    = False
        # Overrides de libellés du header. None → libellés CSV par défaut.
        # Utilisé par le clic-droit "Modifier l'artwork" pour avoir des
        # textes orientés single-card plutôt que "import CSV".
        self._title_override    = title
        self._subtitle_override = subtitle

        # State : choix utilisateur par (idx_groupe, idx_ligne_csv)
        # → valeur = card_image_uuid choisi (ou CHOIX_AUCUN).
        # idx_groupe = index dans self._propositions.
        # idx_ligne_csv = index dans propositions[idx_groupe]["lignes_csv"].
        self._choix: dict = {}

        # Cache CTkImage par card_image_id (évite réouverture PIL)
        self._preview_cache: dict = {}

        # Pool de threads pour DL preview async (limité, comme dialog_anomalies)
        self._preview_executor = ThreadPoolExecutor(
            max_workers=4, thread_name_prefix="ArtworkAltPreview"
        )

        # Variables Tk pour les radios — stockées par (idx_groupe, idx_ligne)
        # afin de pouvoir les manipuler depuis "Appliquer à toutes les raretés"
        self._radio_vars: dict = {}

        self._build_ui()

        # Modal après avoir construit l'UI (sinon erreurs Tk sur certains OS)
        self.protocol("WM_DELETE_WINDOW", self._on_user_close)
        self.after(50, self._set_grab)

    def _set_grab(self):
        try:
            self.grab_set()
            self.focus_force()
        except Exception:
            pass

    # ─────────────────────────────────────────────────────────────────────────
    # Construction UI
    # ─────────────────────────────────────────────────────────────────────────

    def _build_ui(self):
        # ── Header ──
        header = ctk.CTkFrame(self, fg_color=C["bg2"], corner_radius=0, height=80)
        header.pack(fill="x")
        header.pack_propagate(False)

        title_lbl = ctk.CTkLabel(
            header,
            text=self._title_override or "Artworks alternatifs à confirmer",
            font=("Georgia", 18, "bold"),
            text_color=C["gold"],
        )
        title_lbl.pack(anchor="w", padx=20, pady=(14, 2))

        nb_cartes = len(self._propositions)
        nb_lignes = sum(len(p["lignes_csv"]) for p in self._propositions)
        if self._subtitle_override is not None:
            subtitle_text = self._subtitle_override
        else:
            subtitle_text = (
                f"{nb_cartes} carte(s) du CSV utilisent des artworks "
                f"non référencés ({nb_lignes} ligne(s) CSV concernée(s)). "
                "Choisissez l'artwork correspondant à votre carte physique "
                "pour chaque rareté."
            )
        sub_lbl = ctk.CTkLabel(
            header,
            text=subtitle_text,
            font=("Outfit", 11),
            text_color=C["text2"],
            wraplength=900,
            justify="left",
        )
        sub_lbl.pack(anchor="w", padx=20, pady=(0, 12))

        # ── Séparateur ──
        ctk.CTkFrame(self, height=1, fg_color=C["border"],
                     corner_radius=0).pack(fill="x")

        # ── Zone scrollable ──
        self._scroll = ctk.CTkScrollableFrame(
            self, fg_color=C["bg2"], corner_radius=0,
        )
        self._scroll.pack(fill="both", expand=True, padx=12, pady=8)

        for idx, groupe in enumerate(self._propositions):
            self._render_groupe_carte(idx, groupe)

        # ── Footer ──
        ctk.CTkFrame(self, height=1, fg_color=C["border"],
                     corner_radius=0).pack(fill="x")

        footer = ctk.CTkFrame(self, fg_color=C["bg2"], corner_radius=0, height=64)
        footer.pack(fill="x")
        footer.pack_propagate(False)

        secondary_button(
            footer, "Annuler",
            command=self._on_user_close,
            width=120,
        ).pack(side="right", padx=(8, 16), pady=12)

        gold_button(
            footer, "Confirmer la sélection",
            command=self._confirmer,
            width=200,
        ).pack(side="right", padx=4, pady=12)

        # Hint à gauche
        self._lbl_resume = ctk.CTkLabel(
            footer,
            text=self._calculer_resume(),
            font=("Outfit", 10),
            text_color=C["text3"],
        )
        self._lbl_resume.pack(side="left", padx=16, pady=12)

    def _render_groupe_carte(self, idx_groupe: int, groupe: dict):
        """Rend une carte (= 1 set_code = 1 nom). Les raretés CSV sont en
        dessous, chacune avec son propre choix d'artwork."""
        card_frame = ctk.CTkFrame(
            self._scroll, fg_color=C["bg3"],
            border_color=C["border"], border_width=1,
            corner_radius=8,
        )
        card_frame.pack(fill="x", pady=(4, 8), padx=2)

        # ── En-tête carte ──
        head = ctk.CTkFrame(card_frame, fg_color="transparent")
        head.pack(fill="x", padx=14, pady=(10, 6))

        nom_principal = groupe.get("name_fr") or groupe.get("name") or "?"
        nom_secondaire = (
            groupe.get("name") if (groupe.get("name_fr") and groupe.get("name"))
            else ""
        )

        ctk.CTkLabel(
            head, text=nom_principal,
            font=("Outfit", 14, "bold"),
            text_color=C["text"],
        ).pack(anchor="w")

        meta_text = (
            f"{groupe.get('set_code_local', '')}  ·  "
            f"classeur {groupe.get('classeur', '')}"
        )
        if nom_secondaire and nom_secondaire != nom_principal:
            meta_text = f"({nom_secondaire})  ·  " + meta_text

        ctk.CTkLabel(
            head, text=meta_text,
            font=("JetBrains Mono", 10),
            text_color=C["text3"],
        ).pack(anchor="w", pady=(2, 0))

        # ── Bandeau d'artworks proposés (preview row) ──
        propositions = groupe.get("propositions", [])
        artworks_existants = groupe.get("artworks_existants", [])

        prev_section = ctk.CTkFrame(card_frame, fg_color="transparent")
        prev_section.pack(fill="x", padx=14, pady=(4, 8))

        ctk.CTkLabel(
            prev_section,
            text=f"Artworks proposés ({len(propositions)}) :",
            font=("Outfit", 11, "bold"),
            text_color=C["text2"],
        ).pack(anchor="w", pady=(0, 4))

        prev_row = ctk.CTkFrame(prev_section, fg_color="transparent")
        prev_row.pack(fill="x")

        # Existants à gauche (en gris, désactivés visuellement)
        if artworks_existants:
            for art_ex in artworks_existants:
                self._render_preview_box(
                    prev_row, art_ex, label="Déjà importé",
                    desaturate=True, image_id=art_ex.get("card_image_id"),
                    image_url=None, image_url_small=None,
                )

        # Propositions à droite
        for prop in propositions:
            self._render_preview_box(
                prev_row, prop, label="Proposé",
                desaturate=False, image_id=prop.get("card_image_id"),
                image_url=prop.get("card_image_url"),
                image_url_small=prop.get("card_image_small"),
            )

        # ── Liste des raretés CSV (1 ligne par rareté) ──
        ctk.CTkFrame(card_frame, height=1, fg_color=C["border"],
                     corner_radius=0).pack(fill="x", padx=14, pady=(2, 6))

        rar_section = ctk.CTkFrame(card_frame, fg_color="transparent")
        rar_section.pack(fill="x", padx=14, pady=(2, 8))

        ctk.CTkLabel(
            rar_section,
            text=f"Raretés CSV concernées ({len(groupe['lignes_csv'])}) :",
            font=("Outfit", 11, "bold"),
            text_color=C["text2"],
        ).pack(anchor="w", pady=(0, 4))

        for idx_ligne, ligne in enumerate(groupe["lignes_csv"]):
            self._render_ligne_csv(
                rar_section, idx_groupe, idx_ligne, ligne, propositions,
            )

        # ── Bouton "appliquer à toutes les raretés" ──
        if len(groupe["lignes_csv"]) > 1 and propositions:
            btn_row = ctk.CTkFrame(card_frame, fg_color="transparent")
            btn_row.pack(fill="x", padx=14, pady=(0, 12))
            secondary_button(
                btn_row,
                "📋 Appliquer le choix de la 1ʳᵉ rareté à toutes",
                command=lambda i=idx_groupe: self._appliquer_choix_a_toutes(i),
                width=320,
            ).pack(anchor="w")

    def _render_preview_box(self, parent, art: dict, label: str,
                            desaturate: bool, image_id,
                            image_url, image_url_small):
        """Rend une vignette d'artwork (existant ou proposé), avec son label
        sous l'image. Pas de logique radio — c'est purement informatif. La
        sélection se fait par radio dans la liste des raretés en dessous."""
        box = ctk.CTkFrame(parent, fg_color="transparent")
        box.pack(side="left", padx=(0, 10))

        prev_frame = ctk.CTkFrame(
            box, width=PREVIEW_W, height=PREVIEW_H,
            fg_color=C["bg2"],
            border_color=C["border2"] if not desaturate else C["text3"],
            border_width=1,
            corner_radius=4,
        )
        prev_frame.pack()
        prev_frame.pack_propagate(False)

        prev_lbl = ctk.CTkLabel(
            prev_frame, text="⋯", image=None,
            font=("Segoe UI", 16),
            text_color=C["text3"],
        )
        prev_lbl.place(x=0, y=0, relwidth=1, relheight=1)

        # Téléchargement (ou cache → disque) async
        if image_id:
            self._charger_preview_async(
                image_id=image_id,
                url_primary=image_url,
                url_fallback=image_url_small,
                label=prev_lbl,
            )

        info_text = f"#{image_id or '?'}\n{label}"
        ctk.CTkLabel(
            box, text=info_text,
            font=("Outfit", 9),
            text_color=C["text3"] if desaturate else C["text2"],
            justify="center",
        ).pack(pady=(3, 0))

    def _render_ligne_csv(self, parent, idx_groupe: int, idx_ligne: int,
                          ligne: dict, propositions: list[dict]):
        """Une ligne CSV (= 1 rareté) avec ses radio d'artworks."""
        row = ctk.CTkFrame(parent, fg_color="transparent")
        row.pack(fill="x", pady=2)

        # Description rareté + qty
        desc_parts = [f"[{ligne.get('rarete_code', '?')}]"] if ligne.get("rarete_code") else []
        if ligne.get("rarete_full"):
            desc_parts.append(ligne["rarete_full"])
        # Quantité affichée seulement si > 0 — évite "× 0 ex" parasite
        # dans le contexte clic-droit "Modifier l'artwork" où qty_csv=0.
        qty = ligne.get("qty_csv", 0) or 0
        if qty > 0:
            desc_parts.append(f"× {qty} ex")
        if ligne.get("qualite_csv"):
            desc_parts.append(ligne["qualite_csv"])
        if ligne.get("edition_csv"):
            desc_parts.append(str(ligne["edition_csv"]))
        desc_text = "  ·  ".join(desc_parts) if desc_parts else "(slot rareté)"

        ctk.CTkLabel(
            row, text=desc_text,
            font=("JetBrains Mono", 10),
            text_color=C["text"],
            width=320, anchor="w",
        ).pack(side="left", padx=(0, 8))

        # Radios — un par proposition + une option "Ne pas importer"
        radios_frame = ctk.CTkFrame(row, fg_color="transparent")
        radios_frame.pack(side="left", fill="x", expand=True)

        # Variable Tk associée à cette ligne
        var = ctk.StringVar(value=CHOIX_AUCUN)
        self._radio_vars[(idx_groupe, idx_ligne)] = var
        self._choix[(idx_groupe, idx_ligne)] = CHOIX_AUCUN

        def on_change(g=idx_groupe, l=idx_ligne, v=var):
            self._choix[(g, l)] = v.get()
            self._update_resume()

        # Option "ne pas importer"
        ctk.CTkRadioButton(
            radios_frame, text="Ne pas importer",
            variable=var, value=CHOIX_AUCUN,
            command=on_change,
            font=("Outfit", 10),
            text_color=C["text3"],
            fg_color=C["text3"], hover_color=C["bg_hover"],
            border_color=C["border2"], border_width_unchecked=2,
            radiobutton_width=16, radiobutton_height=16,
        ).pack(side="left", padx=4)

        for prop in propositions:
            uuid_prop = prop.get("card_image_uuid", "") or ""
            id_prop   = prop.get("card_image_id", "?")
            if not uuid_prop:
                # Sans uuid stable, on ne peut pas faire un choix fiable —
                # on saute (ne devrait jamais arriver avec card_images bien
                # peuplé).
                continue
            ctk.CTkRadioButton(
                radios_frame, text=f"#{id_prop}",
                variable=var, value=uuid_prop,
                command=on_change,
                font=("Outfit", 10),
                text_color=C["text"],
                fg_color=C["gold"], hover_color=C["gold_hover"],
                border_color=C["gold_dim"], border_width_unchecked=2,
                radiobutton_width=16, radiobutton_height=16,
            ).pack(side="left", padx=4)

    # ─────────────────────────────────────────────────────────────────────────
    # Téléchargement preview async (pattern aligné sur dialog_anomalies)
    # ─────────────────────────────────────────────────────────────────────────

    def _charger_preview_async(self, image_id, url_primary, url_fallback,
                               label: ctk.CTkLabel):
        """Charge la miniature : cache → disque → téléchargement async."""
        if not image_id:
            return

        # 1) Cache mémoire
        cached = self._preview_cache.get(image_id)
        if cached is not None:
            self._afficher_image_dans_label(image_id, None, label, cached)
            return

        # 2) Fichier déjà sur disque (pool partagé img/small/{id}.jpg)
        path = os.path.join(IMAGES_SMALL_FOLDER, f"{image_id}.jpg")
        if os.path.exists(path):
            self._afficher_image_dans_label(image_id, path, label, None)
            return

        # 3) URL primaire alignée sur le téléchargement classeur
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

        # 4) DL async dans le pool
        def worker():
            try:
                from module.img_dl.telechargement_service import TelechargementService
                os.makedirs(IMAGES_SMALL_FOLDER, exist_ok=True)
                service = TelechargementService()
                try:
                    service.telecharger_image(url_eff, path)
                except Exception:
                    pass
                if (not os.path.exists(path)
                        and url_fallback
                        and url_fallback != url_eff):
                    try:
                        service.telecharger_image(url_fallback, path)
                    except Exception:
                        pass
                if self._destroyed:
                    return
                if os.path.exists(path):
                    try:
                        self.after(
                            0,
                            self._afficher_image_dans_label,
                            image_id, path, label, None,
                        )
                    except Exception:
                        pass
            except Exception:
                pass

        try:
            self._preview_executor.submit(worker)
        except Exception:
            # Pool fermé (dialog clos) — on abandonne silencieusement
            pass

    def _afficher_image_dans_label(self, image_id, path, label, ctk_img):
        """Charge depuis disque ou utilise une CTkImage cachée, puis affiche."""
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
                    (PREVIEW_W, PREVIEW_H), Image.LANCZOS
                )
                img = ctk.CTkImage(pil, size=(PREVIEW_W, PREVIEW_H))
                self._preview_cache[image_id] = img
            if img is not None:
                label.configure(image=img, text="")
                # Évite GC : référence sur le label
                setattr(label, "_preview_img_ref", img)
        except Exception:
            try:
                label.configure(text="×")
            except Exception:
                pass

    # ─────────────────────────────────────────────────────────────────────────
    # Actions utilisateur
    # ─────────────────────────────────────────────────────────────────────────

    def _appliquer_choix_a_toutes(self, idx_groupe: int):
        """Propage le choix de la 1ʳᵉ rareté du groupe à toutes les autres
        raretés du même groupe."""
        groupe = self._propositions[idx_groupe]
        nb_lignes = len(groupe["lignes_csv"])
        if nb_lignes <= 1:
            return

        first_var = self._radio_vars.get((idx_groupe, 0))
        if first_var is None:
            return
        valeur = first_var.get()

        for idx_ligne in range(1, nb_lignes):
            v = self._radio_vars.get((idx_groupe, idx_ligne))
            if v is not None:
                v.set(valeur)
                self._choix[(idx_groupe, idx_ligne)] = valeur

        self._update_resume()

    def _calculer_resume(self) -> str:
        n_total = sum(len(p["lignes_csv"]) for p in self._propositions)
        n_choisis = sum(
            1 for v in self._choix.values() if v != CHOIX_AUCUN
        )
        return f"{n_choisis}/{n_total} ligne(s) à importer"

    def _update_resume(self):
        if self._destroyed:
            return
        try:
            self._lbl_resume.configure(text=self._calculer_resume())
        except Exception:
            pass

    def _construire_decisions(self) -> list[dict]:
        """Construit la liste de décisions à passer à appliquer_choix_artworks()
        à partir de l'état des radios."""
        decisions: list[dict] = []
        for idx_groupe, groupe in enumerate(self._propositions):
            propositions = {
                p.get("card_image_uuid", ""): p
                for p in groupe.get("propositions", [])
                if p.get("card_image_uuid")
            }
            for idx_ligne, ligne in enumerate(groupe["lignes_csv"]):
                choix = self._choix.get((idx_groupe, idx_ligne), CHOIX_AUCUN)
                if choix == CHOIX_AUCUN or not choix:
                    continue
                prop = propositions.get(choix)
                if not prop:
                    continue
                decisions.append({
                    "classeur":         groupe.get("classeur", ""),
                    "set_code_local":   groupe.get("set_code_local", ""),
                    "rarete_full":      ligne.get("rarete_full", ""),
                    # Artwork choisi
                    "card_image_uuid":  prop.get("card_image_uuid", ""),
                    "card_image_id":    prop.get("card_image_id"),
                    "card_image_url":   prop.get("card_image_url", ""),
                    "card_image_small": prop.get("card_image_small", ""),
                    # Données CSV à appliquer après INSERT
                    "qty_csv":          ligne.get("qty_csv", 0),
                    "qualite_csv":      ligne.get("qualite_csv", ""),
                    "edition_csv":      ligne.get("edition_csv"),
                })
        return decisions

    def _confirmer(self):
        decisions = self._construire_decisions()
        self._validated = True
        # On ferme avant l'appel callback : le caller peut afficher d'autres
        # dialogs sans empilement modal.
        try:
            if self._on_validate:
                self._on_validate(decisions)
        except Exception as e:
            log.warning(f"ArtworkAltDialog._confirmer callback : {e}")
        self._fermer()

    def _on_user_close(self):
        """Annulation explicite (croix ou bouton Annuler)."""
        self._validated = False
        self._fermer()

    def _fermer(self):
        if self._destroyed:
            return
        self._destroyed = True
        # Shutdown du pool (les threads en cours peuvent finir leur DL,
        # mais aucun nouveau ne sera accepté). wait=False pour ne pas
        # bloquer l'UI pendant la fermeture.
        try:
            self._preview_executor.shutdown(wait=False)
        except Exception:
            pass
        try:
            self.grab_release()
        except Exception:
            pass
        try:
            if self._on_close:
                self._on_close(self._validated)
        except Exception as e:
            log.warning(f"ArtworkAltDialog._fermer on_close : {e}")
        try:
            self.destroy()
        except Exception:
            pass


def afficher_dialog_artwork_alt(parent, propositions: list[dict],
                                on_validate=None, on_close=None,
                                title: str | None = None,
                                subtitle: str | None = None):
    """Helper public : ouvre l'ArtworkAltDialog et le rend modal.

    Args:
        parent       : widget parent.
        propositions : sortie de lister_propositions_artwork_alt() ou
                       sortie de lister_propositions_pour_carte() emballée
                       dans une liste à 1 élément.
        on_validate  : callback(decisions) appelé à la confirmation.
        on_close     : callback(validated: bool) appelé à la fermeture.
        title        : libellé titre custom (ex "Modifier l'artwork — RA02-EN001").
                       Si None, utilise les libellés CSV par défaut.
        subtitle     : libellé sous-titre custom. Si None, génère le texte
                       CSV "X carte(s) du CSV utilisent…".

    Returns:
        L'instance du dialog (déjà ouverte). None si propositions vide.
    """
    if not propositions:
        return None
    dlg = ArtworkAltDialog(
        parent, propositions=propositions,
        on_validate=on_validate, on_close=on_close,
        title=title, subtitle=subtitle,
    )
    return dlg
