"""
theme.py — Thème global Yu-Gi-Oh! Collection Manager.
Palette spec UI : noir profond (#050507) + or luxe (#D4AF37).
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

import customtkinter as ctk

# ─────────────────────────────────────────────────────────────────────────────
# Palette
# ─────────────────────────────────────────────────────────────────────────────

C = {
    # Fonds
    "bg":          "#050507",
    "bg2":         "#0A0B10",
    "bg3":         "#12141D",
    "bg_card":     "#0A0B10",
    "bg_hover":    "#13151F",

    # Accents
    "gold":        "#D4AF37",
    "gold_hover":  "#F5D061",
    "gold_dim":    "#8B7520",

    # Textes
    "text":        "#FFFFFF",
    "text2":       "#A0A4B8",
    "text3":       "#646982",

    # Sémantiques
    "danger":      "#9D1A2A",   # fond plein des boutons « danger »
    "danger_hover":"#C0202F",   # survol des boutons danger (promu depuis un hardcode)
    "danger_text": "#EF5350",   # texte d'erreur lisible sur fond sombre
    "success":     "#2D8A4E",
    "warning":     "#B07D1A",   # fond plein « avertissement »
    "warning_text":"#F5C26B",   # texte d'avertissement (amber) lisible sur fond sombre

    # Bordures (simulées en hex avec alpha fixe)
    "border":      "#14161F",   # ≈ blanc 5%
    "border2":     "#1E2030",   # ≈ blanc 10%
    "border_gold": "#3A3010",   # ≈ or 20%

    # Overframe (art étendu OCG) : accent violet, distinct du doré (possédée),
    # du rouge (danger) et du vert (succès).
    "overframe":      "#7C5CFF",
    "overframe_text": "#FFFFFF",

    # Playset (≥ 3 exemplaires d'une même rareté) : accent émeraude, distinct
    # du doré (possédée), du violet (overframe) et du vert succès. Signale que
    # la rareté est complète pour le jeu (3 copies = un playset).
    "playset":        "#1FB873",
    "playset_text":   "#03130C",
}

# ─────────────────────────────────────────────────────────────────────────────
# Polices
# ─────────────────────────────────────────────────────────────────────────────

# Échelle de police globale — valeur entre 0.85 et 1.50.
# Lue depuis app_config.json au chargement du module. L'utilisateur peut
# ajuster ce facteur via l'onglet Options (redémarrage requis pour
# appliquer partout — un redraw live de 200+ widgets en cours d'affichage
# serait coûteux et sujet à bugs).
#
# IMPORTANT: les polices hard-codées dans le projet ("Segoe UI", 9) ne
# sont PAS affectées par FONT_SCALE — seules les polices obtenues via
# scaled_font() ou les clés FONT[...] réagissent. Cette architecture
# permet une migration progressive : on remplace les hard-codes par
# FONT[...] au fil des cycles.

_FONT_SCALE_MIN     = 0.85
_FONT_SCALE_MAX     = 1.50
_FONT_SCALE_DEFAULT = 1.0


def _load_font_scale() -> float:
    """Lit le facteur d'échelle depuis app_config.json.

    Appelé une seule fois au chargement du module. Les changements
    ultérieurs (via save_font_scale) ne s'appliquent qu'au prochain
    démarrage, car FONT[...] est une constante figée.
    """
    try:
        import module.app_config as _cfg
        value = _cfg.get("font_scale", _FONT_SCALE_DEFAULT)
        if isinstance(value, (int, float)):
            return max(_FONT_SCALE_MIN, min(_FONT_SCALE_MAX, float(value)))
    except Exception:
        pass
    return _FONT_SCALE_DEFAULT


def save_font_scale(scale: float) -> float:
    """Sauvegarde le facteur d'échelle. Retourne la valeur clampée.

    Le changement ne sera visible qu'au prochain redémarrage.
    """
    try:
        scale = float(scale)
    except (TypeError, ValueError):
        scale = _FONT_SCALE_DEFAULT
    scale = max(_FONT_SCALE_MIN, min(_FONT_SCALE_MAX, scale))
    try:
        import module.app_config as _cfg
        _cfg.set("font_scale", scale)
    except Exception:
        pass
    return scale


def get_font_scale() -> float:
    """Retourne le facteur d'échelle courant (peut avoir été mis à jour
    par save_font_scale depuis le début du programme)."""
    # Y1 : délégation pour éviter le doublon avec _load_font_scale
    return _load_font_scale()


def font_scale_bounds() -> tuple[float, float]:
    """Retourne (min, max) pour le slider de configuration."""
    return _FONT_SCALE_MIN, _FONT_SCALE_MAX


# Facteur d'échelle chargé une fois au démarrage
FONT_SCALE = _load_font_scale()


def _scaled(size: int) -> int:
    """Applique FONT_SCALE à une taille de police en arrondissant
    à l'entier le plus proche. Taille minimale 7 pour éviter l'illisibilité
    complète même à l'échelle 0.85.

    Utilise la constante FONT_SCALE (figée au chargement) — utilisé par
    le dict FONT qui est lui-même une constante.
    """
    return max(7, round(size * FONT_SCALE))


def _scaled_live(size: int) -> int:
    """Variante de _scaled qui relit le font_scale à chaque appel.
    Utilisé par scaled_font pour refléter immédiatement un changement
    via save_font_scale sur les widgets créés après."""
    return max(7, round(size * get_font_scale()))


def scaled_font(family: str, size: int, weight: str | None = None):
    """Helper pour définir une police scalée au runtime.

    Usage :
        scaled_font("Outfit", 11)        → ("Outfit", round(11*scale))
        scaled_font("Outfit", 11, "bold") → ("Outfit", round(11*scale), "bold")

    Préféré aux tuples hard-codés ("Outfit", 11) pour les nouveaux widgets
    — permet à l'utilisateur de régler la taille globale via Options.

    Y2 : la lecture du font_scale est live, donc un appel à save_font_scale()
    pendant la session affecte immédiatement les widgets créés après.
    Les widgets créés avant gardent leur taille d'origine jusqu'au
    redémarrage (limitation tkinter — on ne peut pas muter des polices
    déjà assignées sans recréer le widget).
    """
    if weight:
        return (family, _scaled_live(size), weight)
    return (family, _scaled_live(size))


FONT = {
    # Titres — Playfair Display serif, fallback Georgia
    "title_xl": ("Playfair Display", _scaled(28), "bold"),
    "title_lg": ("Playfair Display", _scaled(20), "bold"),
    "title_md": ("Playfair Display", _scaled(16), "bold"),
    "title_sm": ("Georgia",          _scaled(13), "bold"),

    # Corps — Outfit / Segoe UI
    "body_lg":  ("Outfit",   _scaled(13)),
    "body":     ("Outfit",   _scaled(11)),
    "body_sm":  ("Outfit",   _scaled(10)),
    "label":    ("Segoe UI", _scaled(10)),
    "label_sm": ("Segoe UI", _scaled(9)),

    # Monospace — JetBrains Mono / Consolas
    "mono":     ("JetBrains Mono", _scaled(11)),
    "mono_sm":  ("JetBrains Mono", _scaled(9)),
    "mono_xs":  ("JetBrains Mono", _scaled(8)),
}


# ─────────────────────────────────────────────────────────────────────────────
# Setup customtkinter
# ─────────────────────────────────────────────────────────────────────────────

def setup_ctk():
    """Configure le thème global customtkinter."""
    ctk.set_appearance_mode("dark")
    ctk.set_default_color_theme("dark-blue")

