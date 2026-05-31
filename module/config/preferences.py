"""
preferences.py — Préférences utilisateur centralisées.

Stockées dans app_config.json via module.app_config (cache mémoire,
invalidation propre sur set). Expose 3 préférences :

  - grille_defaut (colonnes, lignes)  — utilisée quand un classeur n'a pas
    ses propres cols/lignes dans sa table meta.
  - ordre_tri_criteres (list[str])    — ordre des critères de tri appliqués
    par sort_cartes(). Valeurs possibles : "numero", "rarete", "artwork".
  - affichage_une_rarete_par_artwork (bool) — si True, le visualiseur de
    classeur n'affiche qu'une carte par (set_code, rang_artwork), en
    gardant la rareté la plus rare. Filtre purement visuel : la BDD reste
    intacte et l'export Scanflip continue de tout sortir.

Les getters retournent toujours une valeur valide (fallback sur défaut) :
pas besoin de garde-fous chez les appelants.
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

import module.app_config as _cfg

# ─────────────────────────────────────────────────────────────────────────────
# Grille par défaut
# ─────────────────────────────────────────────────────────────────────────────

_GRILLE_KEY       = "grille_defaut"
_GRILLE_MIN       = 3
_GRILLE_MAX       = 10
_GRILLE_DEFAULT   = (3, 3)   # colonnes, lignes


def _clamp_grille(cols: int, rows: int) -> tuple[int, int]:
    """Garde-fous : valeurs entre 3 et 10 inclus."""
    try:
        c = int(cols)
    except (TypeError, ValueError):
        c = _GRILLE_DEFAULT[0]
    try:
        r = int(rows)
    except (TypeError, ValueError):
        r = _GRILLE_DEFAULT[1]
    c = max(_GRILLE_MIN, min(_GRILLE_MAX, c))
    r = max(_GRILLE_MIN, min(_GRILLE_MAX, r))
    return c, r


def get_grille_defaut() -> tuple[int, int]:
    """Retourne (colonnes, lignes) par défaut — 3×3 si jamais sauvegardé."""
    stored = _cfg.get(_GRILLE_KEY, None)
    if isinstance(stored, list) and len(stored) == 2:
        return _clamp_grille(stored[0], stored[1])
    if isinstance(stored, dict):
        return _clamp_grille(stored.get("cols", 3), stored.get("rows", 3))
    return _GRILLE_DEFAULT


def save_grille_defaut(cols: int, rows: int) -> tuple[int, int]:
    """Sauvegarde grille par défaut après clamping. Retourne la valeur clampée."""
    c, r = _clamp_grille(cols, rows)
    _cfg.set(_GRILLE_KEY, [c, r])
    return c, r


def grille_bounds() -> tuple[int, int]:
    """Retourne (min, max) pour les colonnes/lignes."""
    return _GRILLE_MIN, _GRILLE_MAX


# ─────────────────────────────────────────────────────────────────────────────
# Ordre des critères de tri
# ─────────────────────────────────────────────────────────────────────────────

_TRI_KEY       = "ordre_tri_criteres"
_TRI_CRITERES  = ("numero", "rarete", "artwork")
_TRI_DEFAULT   = ["numero", "artwork", "rarete"]

_TRI_LABELS = {
    "numero":  "Numéro de carte",
    "rarete":  "Rareté",
    "artwork": "Artwork (A avant B)",
}


def _clean_criteres(lst) -> list[str]:
    """Valide et normalise une liste de critères. Retourne une liste qui
    contient exactement _TRI_CRITERES (dans n'importe quel ordre)."""
    if not isinstance(lst, list):
        return list(_TRI_DEFAULT)
    seen = []
    for item in lst:
        if isinstance(item, str) and item in _TRI_CRITERES and item not in seen:
            seen.append(item)
    # Compléter avec les critères manquants (dans leur ordre par défaut)
    for crit in _TRI_CRITERES:
        if crit not in seen:
            seen.append(crit)
    return seen


def get_ordre_tri() -> list[str]:
    """Retourne l'ordre des critères tel que sauvegardé (ou défaut)."""
    stored = _cfg.get(_TRI_KEY, None)
    return _clean_criteres(stored) if stored is not None else list(_TRI_DEFAULT)


def save_ordre_tri(criteres: list[str]) -> list[str]:
    """Sauvegarde l'ordre après validation. Retourne la valeur normalisée."""
    clean = _clean_criteres(criteres)
    _cfg.set(_TRI_KEY, clean)
    return clean


def get_tri_label(critere: str) -> str:
    """Retourne le libellé humain d'un critère."""
    return _TRI_LABELS.get(critere, critere)



# ─────────────────────────────────────────────────────────────────────────────
# Affichage : n'afficher qu'une rareté par numéro+artwork
# ─────────────────────────────────────────────────────────────────────────────
#
# Si activé, dans chaque classeur on n'affiche qu'UNE seule carte par groupe
# (set_code, rang_artwork). La carte affichée est celle avec la rareté la
# plus rare (priorité la plus élevée dans rarity_config.json).
#
# Exemple : RA02-EN001 existe en 7 raretés (Common, Rare, Super Rare, Ultra
# Rare, Secret Rare, Quarter Century, Prismatic). Avec l'option active,
# seule la version la plus rare apparaît dans le visualiseur. Pour les
# cartes avec artwork alternatif, le filtrage est fait par groupe artwork :
# RA02-EN001 (Art A) garde sa rareté la plus rare, RA02-EN001 (Art B) garde
# la sienne séparément.
#
# Les autres raretés restent en base de données et s'exportent normalement
# via Scanflip — le filtre est purement visuel.
#
# ─────────────────────────────────────────────────────────────────────────────
# Filtre raretés affichées par carte+artwork
# ─────────────────────────────────────────────────────────────────────────────
#
# Anciennement booléen (`affichage_une_rarete_par_artwork`) : True = ne garder
# qu'1 rareté par (set_code, art_rank), False = toutes. Maintenant entier N :
#   N = 0 → toutes les raretés affichées (équiv. ancien False)
#   N ≥ 1 → garder les N plus rares par (set_code, art_rank)
#
# Permet à l'utilisateur de choisir précisément combien de raretés afficher
# (ex: 3 pour les 3 plus rares dans les sets RA02 qui en proposent 7).
#
# Filtre PUREMENT visuel : la BDD reste intacte, l'export Scanflip continue
# de tout sortir. Les classeurs peuvent override cette valeur globale via
# leur table meta (cf. creation_classeur_service.get_n_raretes_override).
#
# Backward compat : ancien key `affichage_une_rarete_par_artwork` (bool) est
# lu en fallback si le nouveau key n'est pas encore présent. On migre
# transparent : True → 1, False → 0.
#
_N_RARETES_KEY     = "affichage_n_raretes_par_artwork"
_N_RARETES_DEFAULT = 0
_N_RARETES_MIN     = 0
_N_RARETES_MAX     = 20  # cap raisonnable : aucun set TCG n'a >20 raretés
_OLD_BOOL_KEY      = "affichage_une_rarete_par_artwork"


def n_raretes_bounds() -> tuple[int, int]:
    """Retourne (min, max) pour les sliders/entries d'UI."""
    return _N_RARETES_MIN, _N_RARETES_MAX


def get_n_raretes_par_artwork() -> int:
    """Retourne le N global : nombre de raretés affichées par carte+artwork.

    0 = toutes ; N ≥ 1 = les N plus rares.

    Backward compat : si l'ancien key bool existe et le nouveau pas encore,
    on lit le bool et on le mappe (True → 1, False → 0). La sauvegarde
    suivante par save_n_raretes_par_artwork écrira sous le nouveau key.
    """
    val_new = _cfg.get(_N_RARETES_KEY, None)
    if isinstance(val_new, int) and not isinstance(val_new, bool):
        return max(_N_RARETES_MIN, min(_N_RARETES_MAX, val_new))
    # Fallback : ancien booléen
    val_old = _cfg.get(_OLD_BOOL_KEY, None)
    if isinstance(val_old, bool):
        return 1 if val_old else 0
    if isinstance(val_old, int):
        return 1 if val_old else 0
    return _N_RARETES_DEFAULT


def save_n_raretes_par_artwork(n: int) -> int:
    """Sauvegarde la valeur globale (clampée). Retourne la valeur effective."""
    try:
        n = int(n)
    except (TypeError, ValueError):
        n = _N_RARETES_DEFAULT
    n = max(_N_RARETES_MIN, min(_N_RARETES_MAX, n))
    _cfg.set(_N_RARETES_KEY, n)
    return n


# ── Wrappers de rétrocompatibilité (anciens callers booléens) ────────────────
# Conservés pour ne pas casser l'API en cas de caller oublié. Tous les
# nouveaux callers utilisent get_n_raretes_par_artwork directement.



# ─────────────────────────────────────────────────────────────────────────────
# Inclusion des sets OCG japonais (LOCH, etc.)
# ─────────────────────────────────────────────────────────────────────────────
#
# Par défaut désactivé : l'application reste centrée sur les sets TCG (langues
# 'en' et 'eu' dans set_locales). Quand activée, cette préférence :
#
#   - rend visibles les sets OCG-only japonais (ex: LOCH) dans la liste
#     de création de classeurs
#   - rend disponible la création d'une version japonaise pour les sets
#     existant aussi en TCG (ex: CROS-JP en parallèle de CROS)
#   - inclut les sets japonais dans le scan d'anomalies
#   - permet la résolution d'image de booster pour les sets japonais
#
# Le format de dossier pour un classeur OCG-JP est `XXX-JP/` (avec suffixe
# explicite), distinguant ainsi le classeur OCG-JP d'un éventuel classeur
# TCG du même set (`XXX/`).
#
# La préférence n'affecte que la création de classeurs et le scan : les
# classeurs OCG existants restent ouvrables et fonctionnels même si on
# décoche la préférence après leur création.
#
_INCLURE_OCG_JP_KEY     = "inclure_sets_ocg_jp"
_INCLURE_OCG_JP_DEFAULT = True


def get_inclure_ocg_jp() -> bool:
    """Les sets OCG japonais sont désormais TOUJOURS inclus (plus d'activation).

    Conservé en fonction (et non supprimé) pour que tous les appelants existants
    — get_langues_locales_actives(), scan d'anomalies, listes de création,
    résolution d'images booster — continuent de fonctionner sans modification.
    La valeur stockée éventuelle (ancienne préférence) est ignorée.
    """
    return True


def save_inclure_ocg_jp(active: bool) -> bool:
    """No-op conservé pour compatibilité : l'inclusion OCG-JP est forcée.

    Retourne toujours True. La valeur stockée n'a plus d'effet.
    """
    return True


# ─────────────────────────────────────────────────────────────────────────────
# Suffixes OCG reconnus dans les noms de dossiers et set_codes
# ─────────────────────────────────────────────────────────────────────────────
#
# Liste exhaustive des codes-langue OCG (par opposition aux codes-langue TCG
# 'EN', 'FR', 'DE', 'IT', 'PT', 'SP', 'EU' qui sont gérés par le pipeline
# standard).
#
# - JP / JA : japonais (Konami emploie indifféremment les deux notations,
#             on accepte les deux pour robustesse)
# - KR / KO : coréen
# - AE      : asia-english (Asia-region pour pays asiatiques anglophones)
# - SC / TC : chinois simplifié / chinois traditionnel
#
# Cette liste sert à :
#   1. Détecter qu'un classeur est OCG (suffixe de dossier `LOCH-JP/`)
#   2. Détecter qu'un set_code est OCG (`LOCH-JP001`) pour ne pas appliquer
#      les transformations TCG (ex: _code_to_fr à l'export)
#   3. Empêcher la migration "set_codes legacy → EN" sur ces classeurs
#
# Tous les suffixes sont en MAJUSCULES, comparaison case-insensitive côté
# appelants.
#
_OCG_SUFFIXES = ("JP", "JA", "KR", "KO", "AE", "SC", "TC")


def get_ocg_suffixes() -> tuple[str, ...]:
    """Retourne le tuple des suffixes OCG reconnus (en MAJUSCULES)."""
    return _OCG_SUFFIXES


def a_suffixe_ocg(code_ou_classeur: str) -> bool:
    """True si la chaîne se termine par `-XX` où XX est un suffixe OCG.

    Accepte aussi bien un nom de classeur (`LOCH-JP`) qu'un set_code complet
    en se basant sur la dernière séparation `-XX` ou `-XX###`.

    Exemples :
        a_suffixe_ocg('LOCH-JP')      → True
        a_suffixe_ocg('LOCH-JP001')   → True
        a_suffixe_ocg('CROS')         → False
        a_suffixe_ocg('CROS-EN001')   → False
        a_suffixe_ocg('')             → False
    """
    if not code_ou_classeur:
        return False
    s = code_ou_classeur.strip().upper()
    # Cas 1 : forme `XXX-YY` (nom de classeur) — split sur le dernier '-'
    if "-" in s:
        suffixe_brut = s.rsplit("-", 1)[1]
        # Le suffixe peut être pur (JP) ou contenir un numéro (JP001).
        # On extrait la partie alphabétique de tête.
        i = 0
        while i < len(suffixe_brut) and suffixe_brut[i].isalpha():
            i += 1
        suffixe_alpha = suffixe_brut[:i]
        if suffixe_alpha in _OCG_SUFFIXES:
            return True
    return False


# ─────────────────────────────────────────────────────────────────────────────
# Langues locales actives (filtre SQL set_locales.language)
# ─────────────────────────────────────────────────────────────────────────────

def get_langues_locales_actives() -> tuple[str, ...]:
    """Retourne le tuple des codes-langue à filtrer dans les requêtes SQL
    sur `set_locales.language`.

    Toujours ('en', 'eu') comme base TCG. Si la préférence OCG-JP est
    activée, on ajoute 'ja'. Le résultat est utilisable directement comme
    paramètres d'un `IN (?,?,?)` SQL.

    Évolutif : si demain une préférence "Inclure OCG coréen" est ajoutée,
    on étendra ce helper pour ajouter 'ko'/'kr' selon la convention de
    YGOJSON. Tous les filtres SQL du projet doivent passer par ici plutôt
    que de coder leur propre liste.
    """
    base = ("en", "eu")
    if get_inclure_ocg_jp():
        return base + ("jp",)
    return base
