"""
dialog_anomalies.py — Dialog détection d'anomalies d'artwork.

Évolutions majeures (cycle 2) :
  1. PREVIEW d'image : chaque ligne affiche un thumbnail (72×105 px) de
     l'artwork alternatif qui sera installé si l'anomalie est corrigée.
     Les images sont chargées du disque si présentes (img/small/{id}.jpg),
     sinon téléchargées en arrière-plan via un pool limité à 4 threads.
     Cache CTkImage par image_id pour éviter de rouvrir PIL à chaque render.

  2. MULTI-SÉLECTION : une CTkCheckBox à gauche de chaque ligne. Boutons
     globaux "Tout cocher / décocher" et "✅ Corriger la sélection (N)".
     Le bouton "Tout corriger" existant est conservé.

  3. DÉCLENCHEMENT DU TÉLÉCHARGEMENT : après correction (individuelle, sélection
     ou globale), la tâche FileAttenteClasseur(prefix) est ajoutée. Le worker
     détecte les nouvelles entrées DB et télécharge les artworks manquants.
     Le cache PIL est vidé pour que l'écran classeur affiche les nouvelles
     images dès qu'elles sont sur disque (voir EcranClasseur._poll_dl_progress).
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
from module.ui.composants import gold_button, secondary_button, icon_button
from module.centralisation_dossier import IMAGES_SMALL_FOLDER
from module.logger_app import log


# Ratio carte YGO : 59:86
PREVIEW_W = 72
PREVIEW_H = int(PREVIEW_W * 86 / 59)   # ≈ 105 px


class DialogAnomalies(ctk.CTkToplevel):
    def __init__(self, parent, classeur_code: str, on_update=None,
                 set_code_filter: str | None = None):
        """Dialog de scan/correction des anomalies d'artwork.

        Args:
            parent           : widget parent.
            classeur_code    : code du classeur (ex 'SDWD'). Détermine le
                               préfixe de filtrage à l'affichage.
            on_update        : callback appelé après correction.
            set_code_filter  : si fourni (ex 'SDWD-EN001'), filtre les
                               anomalies sur ce set_code uniquement —
                               utilisé par le menu contextuel "Modifier
                               l'artwork" du clic droit sur une carte.
                               Le scan complet de la base est toujours
                               effectué (cardinfo.db change rarement),
                               mais l'affichage est restreint.
        """
        super().__init__(parent)
        # Titre de fenêtre adapté au mode (utile dans la barre des tâches
        # et dans alt-tab pour distinguer entre dialog globale et ciblée)
        if set_code_filter:
            self.title(f"Modifier l'artwork — {set_code_filter}")
        else:
            self.title("Anomalies d'artwork")
        self.geometry("820x620")
        self.configure(fg_color=C["bg2"])
        self.attributes("-topmost", True)
        self.grab_set()

        self._code             = classeur_code
        self._set_code_filter  = set_code_filter
        self._on_update        = on_update
        self._anomalies   = []
        self._selection: set[int]               = set()   # anomalie ids cochées
        self._preview_cache: dict[int, ctk.CTkImage] = {}
        self._preview_executor = ThreadPoolExecutor(
            max_workers=4, thread_name_prefix="AnomaliePreview"
        )
        self._sel_label  = None      # label "N sélectionnée(s)"
        self._btn_sel    = None      # bouton "Corriger la sélection"
        self._destroyed  = False

        self._build()
        self.bind("<Escape>", lambda e: self.destroy())
        self.protocol("WM_DELETE_WINDOW", self.destroy)

    # ── Cycle de vie ───────────────────────────────────────────────────────

    def destroy(self):
        self._destroyed = True
        try:
            # wait=False pour ne pas bloquer l'UI si des DL sont en cours
            self._preview_executor.shutdown(wait=False)
        except Exception:
            pass
        try:
            super().destroy()
        except Exception:
            pass

    # ── Construction UI ────────────────────────────────────────────────────

    def _build(self):
        # Header
        header = ctk.CTkFrame(self, fg_color="transparent")
        header.pack(fill="x", padx=20, pady=(16, 8))

        # Titre adaptatif : si on est sur une carte précise, on l'indique
        # dans le titre pour que l'utilisateur sache exactement sur quoi
        # il agit (utile car le dialog peut être ouvert depuis 2 endroits :
        # bouton navbar = global, clic droit sur carte = ciblé).
        if self._set_code_filter:
            titre = f"🎨 Modifier l'artwork — {self._set_code_filter}"
        else:
            titre = "📷 Anomalies d'artwork"
        ctk.CTkLabel(header, text=titre,
                     font=("Playfair Display", 14, "bold"),
                     text_color=C["text"]).pack(side="left")
        ctk.CTkButton(header, text="✕", width=30, height=30,
                      fg_color="transparent", hover_color=C["bg_hover"],
                      text_color=C["text2"], command=self.destroy,
                      corner_radius=4).pack(side="right")

        # Sous-titre adaptatif
        if self._set_code_filter:
            sous_titre = (
                "Scan des artworks alternatifs disponibles pour cette "
                "carte. Cochez ceux à ajouter au classeur, puis "
                "« Corriger la sélection »."
            )
        else:
            sous_titre = (
                "Scan bidirectionnel des cartes multi-artworks.\n"
                "Cochez des anomalies puis « Corriger la sélection », "
                "ou cliquez « Corriger » ligne par ligne."
            )
        ctk.CTkLabel(
            self, text=sous_titre,
            font=("Outfit", 10), text_color=C["text3"],
            justify="left",
        ).pack(padx=20, anchor="w")

        ctk.CTkFrame(self, height=1, fg_color=C["border"]).pack(fill="x", padx=20, pady=10)

        # Barre d'actions (visible uniquement une fois des anomalies chargées)
        self._actions_bar = ctk.CTkFrame(self, fg_color="transparent")
        # Pas de .pack() ici : affichée dans _render_results()

        # Zone principale
        self._scroll = ctk.CTkScrollableFrame(self, fg_color="transparent")
        self._scroll.pack(fill="both", expand=True, padx=20, pady=(0, 12))

        # État initial : si on est en mode "carte ciblée" (clic droit),
        # on lance directement le scan — pas besoin de demander
        # confirmation, l'utilisateur a déjà cliqué pour cette action
        # précise. En mode global (bouton navbar), on garde le bouton
        # "Lancer le scan" pour que l'utilisateur sache que ça peut
        # prendre quelques secondes.
        if self._set_code_filter:
            self._lancer_scan()
        else:
            self._render_before_scan()

    def _render_before_scan(self):
        for w in self._scroll.winfo_children():
            w.destroy()
        f = ctk.CTkFrame(self._scroll, fg_color="transparent")
        f.pack(expand=True, pady=40)
        ctk.CTkLabel(f, text="⚠", font=("Segoe UI", 40),
                     text_color=C["gold_dim"]).pack()
        ctk.CTkLabel(f, text="Ce scan interroge la base de données locale.",
                     font=("Outfit", 11), text_color=C["text2"]).pack(pady=8)
        gold_button(f, "═══ Lancer le scan ═══",
                    command=self._lancer_scan, width=220).pack(pady=8)

    def _lancer_scan(self):
        for w in self._scroll.winfo_children():
            w.destroy()
        ctk.CTkLabel(self._scroll, text="⏳ Scan en cours…",
                     font=("Outfit", 11), text_color=C["text3"]).pack(pady=40)
        threading.Thread(target=self._scan_worker, daemon=True).start()

    def _scan_worker(self):
        try:
            from module.anomalie.anomalie_service import (
                scanner_et_stocker_anomalies, lire_anomalies,
                lister_artworks_alternatifs_pour_carte,
                CardinfoIncompleteError,
            )
            try:
                scanner_et_stocker_anomalies()
            except CardinfoIncompleteError as ce:
                # Erreur prévisible : la BDD interne n'est pas initialisée.
                # Affichage d'un message explicite avec bouton d'aide.
                msg = str(ce)
                if not self._destroyed:
                    self.after(0, self._render_cardinfo_missing, msg)
                return

            anomalies = lire_anomalies(prefix_filtre=self._code)

            # Mode ciblé (clic droit "Modifier l'artwork") : on ne garde
            # que les anomalies qui touchent CE set_code précis. Le tri
            # par numéro extrait reste cohérent (déjà fait par
            # lire_anomalies). Si le filtre ne renvoie rien, _render_results
            # affichera un message vide approprié.
            if self._set_code_filter:
                anomalies = [
                    a for a in anomalies
                    if (a.get("missing_set_code") or "").strip()
                       == self._set_code_filter
                ]

                # Complément : artworks détectés directement depuis card_images
                # pour ce set_code (couvre les artworks du même slot qui ne sont
                # pas signalés par le scan cross-set car présents dans la même
                # paire set_code+rarity).
                ids_cross_set = {
                    a.get("image_id") for a in anomalies if a.get("image_id")
                }
                try:
                    directs = lister_artworks_alternatifs_pour_carte(
                        self._code, self._set_code_filter
                    )
                    for art in directs:
                        if art.get("image_id") not in ids_cross_set:
                            anomalies.append(art)
                except Exception as e_dir:
                    log.warning(f"_scan_worker: lister_artworks_alternatifs: {e_dir}")

            if not self._destroyed:
                self.after(0, self._render_results, anomalies)
        except Exception as e:
            msg = str(e)
            if not self._destroyed:
                self.after(0, lambda: ctk.CTkLabel(
                    self._scroll, text=f"⚠ Erreur : {msg}",
                    text_color=C["danger_text"]
                ).pack())

    def _render_cardinfo_missing(self, message: str):
        """Affiche un état d'erreur propre quand cardinfo.db n'est pas prête."""
        for w in self._scroll.winfo_children():
            w.destroy()

        f = ctk.CTkFrame(self._scroll, fg_color="transparent")
        f.pack(expand=True, pady=30, padx=20, fill="x")

        ctk.CTkLabel(
            f, text="⚙", font=("Segoe UI", 42),
            text_color=C["gold_dim"],
        ).pack()
        ctk.CTkLabel(
            f, text="Base interne non initialisée",
            font=("Outfit", 13, "bold"), text_color=C["text"],
        ).pack(pady=(8, 4))
        ctk.CTkLabel(
            f, text=message,
            font=("Outfit", 10), text_color=C["text3"],
            justify="left", wraplength=680,
        ).pack(pady=(0, 12))

        gold_button(
            f, "🔧 Initialiser la base maintenant",
            command=self._lancer_init_bdd, width=260,
        ).pack(pady=6)

        secondary_button(
            f, "Fermer", command=self.destroy, width=120,
        ).pack(pady=(8, 0))

    def _lancer_init_bdd(self):
        """Ferme le dialog et lance l'init BDD depuis la racine."""
        try:
            from module.ui.init_window import show_init_window
        except Exception as e:
            log.warning(f"_lancer_init_bdd : import show_init_window : {e}")
            return

        # Fermer le dialog avant de lancer init (show_init_window est modal)
        parent = self.master
        self.destroy()
        try:
            # show_init_window utilise tk._default_root, donc pas besoin de
            # passer le parent explicitement.
            show_init_window()
        except Exception as e:
            log.warning(f"show_init_window : {e}")

    # ── Rendu résultats ────────────────────────────────────────────────────

    def _render_results(self, anomalies: list):
        self._anomalies = anomalies
        # Conserve la sélection aux ids encore présents
        valid_ids = {a["id"] for a in anomalies}
        self._selection = self._selection & valid_ids

        # Rebuild actions bar
        self._rebuild_actions_bar()

        for w in self._scroll.winfo_children():
            w.destroy()

        n  = len(anomalies)
        nk = sum(1 for a in anomalies if a.get("corrige"))

        # En mode ciblé sans aucune anomalie, on affiche un message
        # explicite : c'est un cas tout à fait normal (la grande majorité
        # des cartes n'ont qu'un seul artwork) et l'utilisateur ne doit
        # pas se demander si "ça a marché".
        if self._set_code_filter and n == 0:
            f = ctk.CTkFrame(self._scroll, fg_color="transparent")
            f.pack(expand=True, pady=40, padx=20)
            ctk.CTkLabel(
                f, text="✓",
                font=("Segoe UI", 42), text_color=C["success"],
            ).pack()
            ctk.CTkLabel(
                f, text="Aucun artwork alternatif connu pour cette carte.",
                font=("Outfit", 12, "bold"), text_color=C["text"],
            ).pack(pady=(8, 4))
            ctk.CTkLabel(
                f,
                text=("La base de données ne référence pas d'autre version "
                      "de cette carte dans ce set. Si tu penses qu'il en "
                      "existe une, vérifie qu'elle est bien dans la BDD "
                      "(Options → MAJ BDD)."),
                font=("Outfit", 10), text_color=C["text3"],
                justify="center", wraplength=560,
            ).pack(pady=(0, 12))
            return

        ctk.CTkLabel(
            self._scroll,
            text=f"{n} carte(s) avec artworks multiples  ·  {nk} corrigée(s)",
            font=("Outfit", 11), text_color=C["text3"],
        ).pack(anchor="w", pady=(0, 8))

        for a in anomalies:
            self._render_anomalie_card(a)

        # Bouton "Tout corriger" (conservé, placé sous la liste)
        if any(not a.get("corrige") for a in anomalies):
            gold_button(
                self._scroll, "✅ Tout corriger",
                command=self._corriger_tout, width=180,
            ).pack(pady=12)

    def _rebuild_actions_bar(self):
        """(Re)construit la barre d'actions en haut (tout cocher, corriger sélection)."""
        for w in self._actions_bar.winfo_children():
            w.destroy()

        anomalies_corrigeables = [a for a in self._anomalies if not a.get("corrige")]
        if not anomalies_corrigeables:
            # Rien à corriger → barre inutile
            try:
                self._actions_bar.pack_forget()
            except Exception:
                pass
            return

        # Afficher la barre
        try:
            self._actions_bar.pack(fill="x", padx=20, pady=(0, 8),
                                   before=self._scroll)
        except Exception:
            self._actions_bar.pack(fill="x", padx=20, pady=(0, 8))

        # Bouton cocher/décocher tout
        all_selected = all(a["id"] in self._selection for a in anomalies_corrigeables)
        label = "☐ Tout décocher" if all_selected else "☑ Tout cocher"
        secondary_button(
            self._actions_bar, label,
            command=self._toggle_all_selection, width=130,
        ).pack(side="left")

        # Label compteur
        self._sel_label = ctk.CTkLabel(
            self._actions_bar,
            text=self._selection_summary(),
            font=("Outfit", 10), text_color=C["text3"],
        )
        self._sel_label.pack(side="left", padx=12)

        # Bouton corriger sélection (à droite)
        self._btn_sel = gold_button(
            self._actions_bar,
            f"✅ Corriger la sélection ({len(self._selection)})",
            command=self._corriger_selection, width=220,
        )
        if self._selection:
            self._btn_sel.pack(side="right")

    def _selection_summary(self) -> str:
        n = len(self._selection)
        if n == 0:
            return "Aucune sélection"
        if n == 1:
            return "1 anomalie sélectionnée"
        return f"{n} anomalies sélectionnées"

    def _refresh_actions_bar(self):
        """Met à jour le label compteur et la visibilité du bouton sélection."""
        if self._sel_label is not None:
            try:
                self._sel_label.configure(text=self._selection_summary())
            except Exception:
                pass
        if self._btn_sel is not None:
            try:
                if self._selection:
                    self._btn_sel.configure(
                        text=f"✅ Corriger la sélection ({len(self._selection)})"
                    )
                    self._btn_sel.pack(side="right")
                else:
                    self._btn_sel.pack_forget()
            except Exception:
                pass

    # ── Ligne anomalie (preview + checkbox + infos + bouton) ──────────────

    def _render_anomalie_card(self, anomalie: dict):
        card = ctk.CTkFrame(
            self._scroll, fg_color=C["bg3"],
            border_color=C["border"], border_width=1,
            corner_radius=8,
        )
        card.pack(fill="x", pady=4)

        row = ctk.CTkFrame(card, fg_color="transparent")
        row.pack(fill="x", padx=12, pady=8)

        corrige = bool(anomalie.get("corrige"))
        aid     = anomalie["id"]

        # ── Checkbox (désactivée si déjà corrigée) ────────────────────────
        chk_var = ctk.BooleanVar(value=(aid in self._selection))

        def on_toggle(a=anomalie, var=chk_var):
            if var.get():
                self._selection.add(a["id"])
            else:
                self._selection.discard(a["id"])
            self._refresh_actions_bar()

        chk = ctk.CTkCheckBox(
            row, text="", variable=chk_var,
            command=on_toggle, width=20,
            checkbox_width=18, checkbox_height=18,
            fg_color=C["gold"], hover_color=C["gold_hover"],
            # P1 : border_color passé de "border" (#14161F, quasi identique à
            # bg3) à "gold_dim" (#8B7520) — la case non-cochée avait un
            # contour invisible sur fond sombre. Avec gold_dim, la case reste
            # visible à l'état décoché et prend son fond doré à l'état coché.
            border_color=C["gold_dim"],
            border_width=2,
        )
        if corrige:
            chk.configure(state="disabled")
        chk.pack(side="left", padx=(0, 10))

        # ── Preview image ─────────────────────────────────────────────────
        preview_frame = ctk.CTkFrame(
            row, width=PREVIEW_W, height=PREVIEW_H,
            fg_color=C["bg2"], corner_radius=4,
        )
        preview_frame.pack(side="left", padx=(0, 10))
        preview_frame.pack_propagate(False)

        preview_lbl = ctk.CTkLabel(
            preview_frame, text="⋯", image=None,
            font=("Segoe UI", 18), text_color=C["text3"],
        )
        preview_lbl.place(x=0, y=0, relwidth=1, relheight=1)

        # Charge la preview (cache → disque → DL async)
        self._charger_preview(anomalie, preview_lbl)

        # ── Textes (nom, set_code, art_index) ─────────────────────────────
        infos = ctk.CTkFrame(row, fg_color="transparent")
        infos.pack(side="left", fill="both", expand=True)

        ctk.CTkLabel(
            infos, text=anomalie.get("name", ""),
            font=("Outfit", 13, "bold"), text_color=C["text"],
            anchor="w",
        ).pack(anchor="w")

        ctk.CTkLabel(
            infos,
            text=(f"[{anomalie.get('missing_set_code', '')}] "
                  f"· {anomalie.get('missing_set_rarity', '')}"),
            font=("JetBrains Mono", 11),
            text_color=C["text3"], anchor="w",
        ).pack(anchor="w", pady=(1, 0))

        ctk.CTkLabel(
            infos, text=f"Artwork alternatif #{anomalie.get('art_index', '?')}",
            font=("Outfit", 11), text_color=C["text3"], anchor="w",
        ).pack(anchor="w")

        # ── Statut / bouton corriger ──────────────────────────────────────
        action = ctk.CTkFrame(row, fg_color="transparent")
        action.pack(side="right")

        if corrige:
            ctk.CTkLabel(
                action, text="✅ corrigé",
                font=("Outfit", 12), text_color=C["success"],
            ).pack(padx=4)
        else:
            secondary_button(
                action, "Corriger",
                command=lambda a=anomalie: self._corriger_un(a),
                width=90,
            ).pack(padx=4)

    # ── Preview async ──────────────────────────────────────────────────────

    def _charger_preview(self, anomalie: dict, label: ctk.CTkLabel):
        """Charge la miniature d'un artwork pour l'afficher dans le dialog.

        Cohérence avec le classeur : on télécharge EXACTEMENT la même image
        que celle qui sera utilisée dans le visualiseur de classeur. On passe
        donc par build_image_url() (config_image_source) au lieu de
        image_url_small qui pointerait sur l'artwork-only Yugipedia.

        Conséquences :
          - L'aperçu affiche la carte complète (cadre + texte + artwork),
            strictement identique à ce qui apparaîtra après correction.
          - Le fichier téléchargé est sauvé dans le pool partagé
            img/small/{image_id}.jpg, donc le worker FileAttenteClasseur
            trouvera l'image déjà présente et sautera son téléchargement
            (pas de redondance réseau).
          - Fallback sur image_url_small → image_url si build_image_url()
            retourne None (ex : source YUGIPEDIA configurée mais URL
            Yugipedia spécifique au print indisponible).

        Stratégie : cache mémoire → disque → téléchargement async.
        """
        image_id     = anomalie.get("image_id")
        url_primary  = None
        url_fallback = anomalie.get("image_url_small") or anomalie.get("image_url") or ""

        # URL primaire alignée sur le téléchargement du classeur
        try:
            from module.config_image_source import build_image_url
            url_primary = build_image_url(
                anomalie.get("image_url"), image_id
            )
        except Exception as e:
            log.warning(f"_charger_preview: build_image_url a échoué: {e}")

        effective_url = url_primary or url_fallback

        if not image_id or not effective_url:
            try:
                label.configure(text="?")
            except Exception:
                pass
            return

        # 1) Cache mémoire
        cached = self._preview_cache.get(image_id)
        if cached is not None:
            try:
                label.configure(image=cached, text="")
                # évite le GC
                setattr(label, "_preview_img_ref", cached)
            except Exception:
                pass
            return

        # 2) Fichier sur disque (pool partagé — MÊME emplacement que le classeur)
        path = os.path.join(IMAGES_SMALL_FOLDER, f"{image_id}.jpg")
        if os.path.exists(path):
            self._display_preview_from_path(image_id, path, label)
            return

        # 3) Téléchargement async avec fallback sur l'URL secondaire
        def worker():
            try:
                from module.img_dl.telechargement_service import TelechargementService
                os.makedirs(IMAGES_SMALL_FOLDER, exist_ok=True)
                service = TelechargementService()

                # Essai 1 : URL principale (cohérente avec le classeur)
                try:
                    service.telecharger_image(effective_url, path)
                except Exception:
                    pass

                # Si le DL a échoué ET qu'on a une URL de fallback différente,
                # on retente avec le fallback. Utile si l'image n'existe pas
                # côté YGOPRODeck mais existe côté Yugipedia (rare).
                if (not os.path.exists(path)
                        and url_fallback
                        and url_fallback != effective_url):
                    try:
                        service.telecharger_image(url_fallback, path)
                    except Exception:
                        pass

                if self._destroyed:
                    return
                if os.path.exists(path):
                    # Retour au main thread pour manipuler le widget
                    self.after(
                        0,
                        self._display_preview_from_path,
                        image_id, path, label,
                    )
            except Exception:
                pass

        try:
            self._preview_executor.submit(worker)
        except Exception:
            # Pool shut down (dialog fermé) — on abandonne silencieusement
            pass

    def _display_preview_from_path(self, image_id: int, path: str,
                                   label: ctk.CTkLabel):
        """Charge l'image depuis disque, la convertit en CTkImage, l'affiche."""
        if self._destroyed:
            return
        try:
            # Le label peut avoir été détruit entre-temps
            if not label.winfo_exists():
                return
        except Exception:
            return
        try:
            pil = Image.open(path).convert("RGB").resize(
                (PREVIEW_W, PREVIEW_H), Image.LANCZOS
            )
            img = ctk.CTkImage(pil, size=(PREVIEW_W, PREVIEW_H))
            self._preview_cache[image_id] = img
            label.configure(image=img, text="")
            setattr(label, "_preview_img_ref", img)
        except Exception:
            try:
                label.configure(text="×")
            except Exception:
                pass

    # ── Sélection ──────────────────────────────────────────────────────────

    def _toggle_all_selection(self):
        corrigeables = [a for a in self._anomalies if not a.get("corrige")]
        all_selected = all(a["id"] in self._selection for a in corrigeables)
        if all_selected:
            for a in corrigeables:
                self._selection.discard(a["id"])
        else:
            for a in corrigeables:
                self._selection.add(a["id"])
        # Re-render pour refléter l'état des checkboxes
        self._render_results(self._anomalies)

    # ── Corrections ────────────────────────────────────────────────────────

    def _corriger_un(self, anomalie: dict):
        from module.anomalie.anomalie_service import (
            corriger_anomalie, lire_anomalies,
        )
        _, touche = corriger_anomalie(anomalie)
        self._post_correction([touche] if touche else [])
        if not self._destroyed:
            if self._set_code_filter:
                # Re-scan complet pour rafraîchir anomalies ET artworks directs
                self._lancer_scan()
            else:
                self._render_results(lire_anomalies(prefix_filtre=self._code))

    def _corriger_selection(self):
        if not self._selection:
            return
        from module.anomalie.anomalie_service import (
            corriger_anomalies, lire_anomalies,
        )
        selectionnees = [a for a in self._anomalies
                         if a["id"] in self._selection and not a.get("corrige")]
        if not selectionnees:
            return
        _, touches = corriger_anomalies(selectionnees)
        self._selection.clear()
        self._post_correction(touches)
        if not self._destroyed:
            if self._set_code_filter:
                self._lancer_scan()
            else:
                self._render_results(lire_anomalies(prefix_filtre=self._code))

    def _corriger_tout(self):
        from module.anomalie.anomalie_service import (
            corriger_anomalies, lire_anomalies,
        )
        non_ok = [a for a in self._anomalies if not a.get("corrige")]
        if not non_ok:
            return
        _, touches = corriger_anomalies(non_ok)
        self._post_correction(touches)
        if not self._destroyed:
            if self._set_code_filter:
                self._lancer_scan()
            else:
                self._render_results(lire_anomalies(prefix_filtre=self._code))

    def _post_correction(self, classeurs_touches: list):
        """Déclenche le téléchargement des nouvelles images et notifie le parent.

        corriger_anomalie() insère une nouvelle ligne dans cards(...) avec
        card_image_url/card_image_id pointant vers l'artwork alternatif, mais
        ne télécharge PAS l'image correspondante. On ajoute donc une tâche à
        FileAttenteClasseur : le worker scanne la DB, détecte les entrées
        dont le fichier image est absent de img/small/, et les télécharge.

        Le flag `from_missing_check = True` évite que le worker relance un
        charger_les_cartes() complet à la fin (logique de l'ancien UI).

        Le cache PIL est vidé pour que le prochain rendu du classeur affiche
        les nouvelles images dès qu'elles sont sur disque.
        """
        # Invalide le cache PIL (les nouvelles entrées pointent vers des
        # fichiers qui peuvent arriver pendant que l'écran classeur est ouvert)
        try:
            from module.gestion_img.cache_images import clear_cache
            clear_cache()
        except Exception:
            pass

        # Déclenche le DL pour chaque classeur touché
        if classeurs_touches:
            try:
                from module.img_dl.file_attente_classeur import FileAttenteClasseur
                file = FileAttenteClasseur()
                for prefix in classeurs_touches:
                    tache = file.ajouter(prefix)
                    # Marquer pour que l'UI parente ne boucle pas sur un
                    # rechargement complet automatique (le re-render est géré
                    # par EcranClasseur._poll_dl_progress → transition TERMINE)
                    tache.from_missing_check = True
            except Exception as e:
                log.warning(f"_post_correction : ajout file d'attente : {e}")

        # Prévient le parent (EcranClasseur.charger()) pour recharger
        # les cartes avec les nouvelles lignes (affichées en placeholder
        # tant que l'image n'est pas DL → le _poll_dl_progress fera le reste)
        if self._on_update:
            try:
                self._on_update()
            except Exception as e:
                log.warning(f"_post_correction : callback on_update : {e}")
