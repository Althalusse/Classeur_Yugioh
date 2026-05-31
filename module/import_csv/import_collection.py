"""
import_collection.py — Import CSV de la collection au format Scanflip.

Format de référence : https://scanflip.fr — round-trip exact garanti avec
l'export correspondant (module.export.export_collection).

Comportement :
  - Mode REPLACE : la quantité du CSV remplace celle de la DB (pas additive).
    Workflow type : export → modif sur Scanflip → réimport. La valeur du
    CSV est la source de vérité.
  - Si plusieurs lignes du CSV pointent vers la même carte physique (cas
    LDK2-FRK01 : 3 lignes pour la même rareté/artwork), les quantités du
    CSV sont AGRÉGÉES avant le replace en DB. Cela respecte la sémantique
    Scanflip d'avoir une ligne par instance physique.

Algorithme de matching (par ligne CSV) :
  1. Extension     → identifier le classeur (db_path)
  2. Rareté        → convertir code Scanflip vers nom complet DB
  3. N° Artwork    → résoudre vers le rang artwork (vide=0, "1"=1er alt, …)
  4. Édition+Qualité → lire la colonne non vide parmi 1st/Unlimited/Limited
  5. Matching      → triplet (set_code_normalisé, rang_artwork, rarité)
                     UNIQUE en BDD ; le nom CSV n'entre PAS dans la clé.
  6. UPDATE        → écrire possessed/quantite/qualite/edition

POURQUOI LE NOM N'EST PAS UNE CLÉ DE MATCHING
─────────────────────────────────────────────
Les conventions typographiques diffèrent entre Konami FR (Scanflip),
YGOJSON et YGOPRODeck. Cas réels rencontrés :
  • « Ange 01 » (chiffre zéro, Scanflip) vs « Ange O1 » (lettre O, YGOJSON)
  • « lance Interdite » vs « Lance Interdite » (casse première lettre)
  • « Voeux » vs « Vœux » (ligature œ)
Tous ces cas étaient rejetés à tort par l'ancienne logique alors que le
triplet (set_code, rang_artwork, rarité) suffit à identifier sans ambiguïté
une carte physique. Le nom CSV est désormais conservé uniquement pour le
diagnostic des cartes vraiment introuvables (rapport "non trouvées").

Cas d'erreur gérés :
  - Classeur inexistant      → toutes ses lignes ignorées, listées dans le retour
  - Carte introuvable en DB  → ligne ignorée, ajoutée à `non_trouvees` avec
                               un diagnostic enrichi (set_code absent / rang
                               hors limites / rareté absente)
  - Rareté inconnue          → ligne ignorée + warning
  - Quantité non numérique   → ligne ignorée + warning
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

import csv
import os
import re
from collections import defaultdict
from module.centralisation_dossier import CLASSEUR_FOLDER, CARDINFO_DB, sqlite_ctx
from module.gestion_rarete.raretes_reference import (
    code_to_name_en, code_to_name_fr, is_known_code,
)


# Mapping inverse : colonne CSV qui contient la qualité → code édition DB
COLUMN_TO_EDITION = {
    "1st Edition":      "1st",
    "Unlimited":        "unlimited",
    "Limited / Autre":  "limited",
}

# Limite du nombre de lignes "non trouvées" retournées dans le résumé
# (pour ne pas saturer l'UI sur un import géant qui matcherait mal).
NON_TROUVEES_PREVIEW_MAX = 50

# Limite du nombre de warnings retournés (raretés inconnues, etc.)
WARNINGS_PREVIEW_MAX = 50


# Pattern de conversion code-langue → EN.
# Notre BDD locale stocke les set_code en EN (cf. _build_rows_from_local_db
# qui fait COALESCE(sp_en.set_code, sp_fr.set_code)). Tout code CSV non-EN
# doit donc être converti avant le matching.
#
# Liste des langues alignée sur _LANG_SUFFIXES de ygojson_parser.py + alias.
#
# Le pattern matche : "-LANG[suffixe alphabétique optionnel]NUMÉRO"
# où LANG est dans la liste et NUMÉRO contient au moins un chiffre.
#
# Pourquoi le suffixe alphabétique optionnel ? Beaucoup de produits
# Yu-Gi-Oh! ont une lettre indicateur entre la langue et le numéro de carte
# pour distinguer un sous-deck ou une sous-collection. Cas réels :
#   - Legendary Decks II : LDK2-FRJ01 (Joey), LDK2-FRK01 (Kaiba), LDK2-FRY01 (Yugi)
#   - Legendary Hero Decks : LEHD-FRA01, LEHD-FRB01, LEHD-FRC01
#   - Movie Pack 1 : MVP1-FRG01 (Gold), MVP1-FRS01 (Silver)
#   - BOSH Special Edition : BOSH-FRSE1
# Plus de 115 sets dans cardinfo.db utilisent ce schéma. La lettre du
# sous-deck est CONSERVÉE telle quelle entre les langues — par convention
# Konami, le préfixe alphabétique n'est jamais traduit.
#
# Le pattern NE matche QUE si la langue est suivie d'un suffixe alphabétique
# OU directement d'un chiffre. Donc "DUEA-ENDE1" (set EN avec suffixe DE)
# n'est PAS modifié car "EN" n'est pas dans la liste des langues sources.
_LANG_TO_EN_PATTERN = re.compile(
    r'-(FR|DE|IT|ES|PT|JP|JA|KR|KO|SP|AE|SC|NA|EU|AU|AS|TC|TF|TG)([A-Z]*\d+)',
    re.IGNORECASE,
)


# ─────────────────────────────────────────────────────────────────────────────
# Helpers — décodage d'une ligne CSV
# ─────────────────────────────────────────────────────────────────────────────

def _parse_quantite(value) -> int | None:
    """Retourne int(value) ou None si pas convertible."""
    if value is None or value == "":
        return None
    try:
        n = int(str(value).strip())
        return n if n >= 0 else None
    except (ValueError, TypeError):
        return None


def _decode_edition_et_qualite(row: dict) -> tuple[str | None, str]:
    """
    Lit la colonne d'édition non vide parmi les 3 mutuellement exclusives.

    Returns:
        (edition_code, qualite) où :
          - edition_code ∈ {'1st', 'unlimited', 'limited', None}
          - qualite est la valeur lue (M, NM, EX, ...) ou ""

    Si plusieurs colonnes sont remplies (CSV mal formé), on prend la
    première dans l'ordre de priorité 1st > Unlimited > Limited.
    """
    for col, edition_code in COLUMN_TO_EDITION.items():
        val = (row.get(col) or "").strip()
        if val:
            return edition_code, val
    return None, ""


def _decode_artwork_rank(value) -> int:
    """
    Convertit la valeur 'N° Artwork' en rang artwork (0 = principal,
    1 = 1er alternatif, 2 = 2e, etc.).

    Convention Scanflip : vide → principal (rang 0), "1" → 1er alt (rang 1).

    Robuste aux espaces et aux valeurs vides.
    """
    if value is None:
        return 0
    s = str(value).strip()
    if not s:
        return 0
    try:
        return max(0, int(s))
    except ValueError:
        return 0


# ─────────────────────────────────────────────────────────────────────────────
# Helpers — accès DB
# ─────────────────────────────────────────────────────────────────────────────

def _classeur_existe(code: str) -> bool:
    """True si le dossier+DB du classeur existent."""
    db_path = os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")
    return os.path.isfile(db_path)


def _classeur_db_path(code: str) -> str:
    return os.path.join(CLASSEUR_FOLDER, code, f"{code}.db")


def _build_artwork_index(conn) -> dict:
    """
    Construit l'index (set_code, rang_artwork, rarity) → liste de {rowid, name, name_fr}.

    Cette structure de clé est strictement équivalente à l'identifiant
    physique d'une carte de collection :
      - set_code identifie le tirage (ex 'EGO1-EN006')
      - rang_artwork distingue les artworks alternatifs au sein d'un même
        set_code (0 = principal, 1 = 1er alternatif, etc.)
      - rarity distingue les multi-raretés d'un même tirage (rare cas où
        un set_code apparaît en plusieurs raretés, ex sets Quarter Century)

    Le nom n'entre PAS dans la clé — voir docstring du module pour la
    justification (typographies divergentes entre sources). Il est
    conservé dans la valeur uniquement pour le diagnostic des non-trouvées.

    Calcul du rang d'artwork
    ────────────────────────
    Pour un set_code donné, on collecte tous les card_image_id distincts
    présents en DB, on les trie par valeur croissante, et on attribue
    rang 0, 1, 2… dans l'ordre. Cohérent avec la convention de l'export
    (cf. _get_art_rank dans export_collection.py).

    Note : grouper par `set_code` seul ou par `(name, set_code)` revient
    au même en pratique, car deux cartes physiquement différentes ne
    peuvent partager le même set_code (par définition d'un set_code).
    Grouper par set_code seul est plus robuste face aux divergences de
    nommage entre langues stockées dans le classeur.
    """
    cursor = conn.cursor()

    # Détection schéma : la colonne name_fr peut ne pas exister sur les
    # tout premiers classeurs (avant la migration G1). Robuste aux deux cas.
    cursor.execute("PRAGMA table_info(cards)")
    cols = {c[1] for c in cursor.fetchall()}
    has_name_fr = "name_fr" in cols

    if has_name_fr:
        cursor.execute("""
            SELECT rowid, name, name_fr, set_code, rarity, card_image_id
            FROM cards
        """)
    else:
        cursor.execute("""
            SELECT rowid, name, '' AS name_fr, set_code, rarity, card_image_id
            FROM cards
        """)
    rows = cursor.fetchall()

    # Étape 1 : pour chaque set_code, collecter les card_image_id distincts
    groups: dict = defaultdict(set)
    for rowid, name, name_fr, set_code, rarity, img_id in rows:
        if set_code:
            groups[set_code].add(img_id or 0)

    # Étape 2 : assigner un rang à chaque card_image_id du groupe
    rank_map: dict = {}
    for set_code, ids in groups.items():
        for rank, img_id in enumerate(sorted(ids)):
            rank_map[(set_code, img_id)] = rank

    # Étape 3 : index final (set_code, rank, rarity) → [{rowid, name, name_fr}]
    index: dict = defaultdict(list)
    for rowid, name, name_fr, set_code, rarity, img_id in rows:
        if not set_code:
            continue
        rank = rank_map.get((set_code, img_id or 0))
        if rank is None:
            continue
        index[(set_code, rank, rarity or "")].append({
            "rowid":          rowid,
            "name":           name or "",
            "name_fr":        name_fr or "",
            "rarity_db_str":  rarity or "",  # pour traçabilité dans les
                                              # warnings du fallback 2b
        })

    return index


def _convert_set_code_to_local(code_csv: str, langue_locale: str = "EN") -> str:
    """
    Convertit le code carte CSV (ex 'RA02-FR001') vers le code stocké en DB.

    Notre BDD locale stocke les codes en EN (ex 'RA02-EN001') car
    _build_rows_from_local_db privilégie systématiquement le set_code EN
    via COALESCE. Si l'utilisateur importe un CSV en FR/DE/IT/ES/etc.,
    on convertit le suffixe de langue vers EN pour le matching.

    Le pattern préserve les éventuels sous-préfixes alphabétiques entre
    la langue et le numéro :
      - 'LDK2-FRJ01' → 'LDK2-ENJ01' (Joey deck du box-set Legendary Decks II)
      - 'MVP1-FRG05' → 'MVP1-ENG05' (variante Gold de Movie Pack 1)
      - 'EGO1-FR006' → 'EGO1-EN006' (cas standard, pas de sous-préfixe)
      - 'DUEA-ENDE1' → 'DUEA-ENDE1' (déjà EN, pas modifié)

    Si le code ne contient pas de séparateur typique XXX-LLnnn, retourne tel quel.
    """
    if not code_csv or "-" not in code_csv:
        return code_csv
    if langue_locale == "EN":
        return _LANG_TO_EN_PATTERN.sub(r'-EN\2', code_csv)
    return code_csv


def _generer_variantes_set_code(code_csv: str) -> list[str]:
    """Génère toutes les variantes linguistiques d'un set_code.

    Ex : 'SDWD-FR013' → ['SDWD-EN013', 'SDWD-FR013', 'SDWD-DE013',
                         'SDWD-IT013', 'SDWD-ES013', 'SDWD-PT013',
                         'SDWD-JP013', ...]

    Utile pour rattraper les classeurs legacy qui ont des set_codes mixtes
    en BDD (cf. bug SDWD avant le fix de _build_rows_from_local_db).

    Le code d'origine est inclus dans le résultat. Si le code n'a pas de
    motif langue reconnaissable (ex 'PROMO-001' sans préfixe langue),
    retourne juste [code_csv].

    Pas de garantie d'ordre — utiliser comme un set de candidats à tester.
    """
    if not code_csv or "-" not in code_csv:
        return [code_csv]

    # Trouve le motif -LL[A-Z]*\d+ et génère toutes les variantes.
    match = _LANG_TO_EN_PATTERN.search(code_csv)
    if not match:
        # Code déjà en EN ou format inconnu (ex 'DUEA-ENDE1' déjà EN).
        # Pour les codes EN, on génère quand même les variantes en
        # remplaçant 'EN' par les autres langues.
        en_match = re.search(r'-EN([A-Z]*\d+)', code_csv, re.IGNORECASE)
        if not en_match:
            return [code_csv]
        suffix = en_match.group(1)
        prefix = code_csv[:en_match.start()]
        langues = ["EN", "FR", "DE", "IT", "ES", "PT", "JP", "KR"]
        return [f"{prefix}-{lang}{suffix}" for lang in langues]

    # Cas standard : on a une langue source (FR/DE/...) qu'on traduit
    suffix = match.group(2)  # ex "013" ou "J01" ou "G05"
    prefix = code_csv[:match.start()]
    langues = ["EN", "FR", "DE", "IT", "ES", "PT", "JP", "KR"]
    return [f"{prefix}-{lang}{suffix}" for lang in langues]


def _trouver_rarity_full(code_rarete: str, prefer_lang: str = "en") -> str | None:
    """
    Convertit un code Scanflip (ex 'SCR') vers le nom complet stocké en DB.

    La DB stocke en majorité les noms EN (YGOPRODeck), donc on essaie EN
    en priorité. Si le nom EN est trouvé, on le retourne ; sinon on tente FR.
    """
    if not is_known_code(code_rarete):
        return None
    if prefer_lang == "fr":
        return code_to_name_fr(code_rarete)
    return code_to_name_en(code_rarete)


# ─────────────────────────────────────────────────────────────────────────────
# Lecture CSV
# ─────────────────────────────────────────────────────────────────────────────

def _read_csv_rows(chemin_fichier: str) -> list[dict]:
    """Lit le CSV avec gestion BOM + erreurs encodage."""
    with open(chemin_fichier, "r", encoding="utf-8-sig", newline="") as f:
        reader = csv.DictReader(f)
        return list(reader)


# ─────────────────────────────────────────────────────────────────────────────
# Helpers — diagnostic enrichi via cardinfo.db (artworks globalement connus)
# ─────────────────────────────────────────────────────────────────────────────

def _build_artworks_globaux_index(classeur: str) -> dict:
    """
    Pour les non-trouvées avec rang artwork > 0, on a besoin de savoir si
    l'artwork alternatif EXISTE GLOBALEMENT (côté card_images de cardinfo.db)
    même si YGOPRODeck ne le tagge pas dans ce set précis.

    Cas réel : RA02-EN001 (Chat Sauveteur) en BDD a 7 prints (un par rareté),
    tous pointant vers l'artwork principal. Mais la table card_images globale
    contient bien 2 artworks pour cette carte (passwords 14878871 et
    14878872). YGOPRODeck ne précise pas que le 2e artwork est dispo en RA02.

    Cette fonction interroge cardinfo.db pour récupérer, pour chaque
    set_code utilisé dans le classeur, le nombre total d'artworks que la
    carte associée possède dans la collection mondiale. Ça permet de
    distinguer trois cas dans le diagnostic des non-trouvées :

      - card connue mais artwork alt non taggé sur ce set
        → réparable via le futur lot "Artworks alt à l'import CSV"
      - card avec un seul artwork connu
        → vrai rang invalide, l'utilisateur a saisi un mauvais N° Artwork
      - card inconnue
        → set_code/carte non gérée par cardinfo.db

    Returns:
        dict { set_code (EN) → nb_artworks_connus_globalement }
        Vide si cardinfo.db inaccessible (le diagnostic dégrade gracieusement).

    Pourquoi indexer par set_code et pas par card_uuid : à ce stade du
    code, on connaît juste le set_code CSV. Aller du set_code au card_uuid
    nécessite un JOIN ; on l'inclut donc dans la même requête, et on agrège
    nb_artworks au niveau set_code (puisque dans 99% des cas un set_code
    correspond à une seule card_uuid — les rares cas multi-card sur un
    même set_code sont gérés en prenant le MAX, ce qui reste pertinent
    pour le diagnostic).

    Note d'opération : la requête est en lecture seule et bornée aux
    set_codes utilisés par le classeur, donc le coût est faible même sur
    cardinfo.db (162 Mo). Pas d'index dédié nécessaire.
    """
    if not os.path.isfile(CARDINFO_DB):
        return {}

    db_classeur_path = _classeur_db_path(classeur)
    if not os.path.isfile(db_classeur_path):
        return {}

    # Étape 1 : récupérer tous les set_codes du classeur
    try:
        with sqlite_ctx(db_classeur_path) as conn:
            rows = conn.execute(
                "SELECT DISTINCT set_code FROM cards WHERE set_code IS NOT NULL"
            ).fetchall()
        set_codes = [r[0] for r in rows if r[0]]
    except Exception:
        return {}

    if not set_codes:
        return {}

    # Étape 2 : pour chaque set_code, compter les artworks globaux connus.
    # Pour chaque set_code, on récupère le card_uuid des prints associés,
    # puis on compte les card_images rattachées à ce card_uuid.
    result: dict[str, int] = {}
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            # Limite SQL à 999 placeholders ; on chunke pour les classeurs
            # exceptionnellement gros (peu probable mais défensif).
            CHUNK = 500
            for i in range(0, len(set_codes), CHUNK):
                batch = set_codes[i:i + CHUNK]
                placeholders = ",".join("?" * len(batch))
                rows = conn.execute(f"""
                    SELECT sp.set_code, COUNT(DISTINCT ci.uuid) AS nb_arts
                    FROM set_prints sp
                    LEFT JOIN card_images ci ON ci.card_uuid = sp.card_uuid
                    WHERE sp.set_code IN ({placeholders})
                    GROUP BY sp.set_code
                """, batch).fetchall()
                for set_code, nb in rows:
                    # max() au cas (très rare) où un set_code apparaîtrait
                    # avec plusieurs card_uuid distinctes
                    if nb is not None:
                        result[set_code] = max(result.get(set_code, 0), int(nb))
    except Exception:
        # En cas d'erreur d'accès cardinfo.db, on retourne ce qu'on a déjà
        # récolté (potentiellement vide). Le diagnostic dégradera juste
        # vers le message générique, jamais d'erreur fatale.
        pass

    return result




def _importer_dans_classeur(
    classeur: str,
    rows_csv: list[dict],
    non_trouvees: list,
    warnings: list,
) -> int:
    """
    Importe les lignes CSV dont Extension == classeur.

    Mode REPLACE : pour chaque carte matchée, l'UPDATE écrase la quantité
    existante par celle agrégée du CSV. Si plusieurs lignes du CSV
    pointent vers la même carte physique (set_code+rarity+artwork+edition
    identiques), leurs quantités sont sommées AVANT le replace en DB.

    Args:
        classeur     : code du classeur (ex 'RA02')
        rows_csv     : lignes CSV pré-filtrées sur ce classeur
        non_trouvees : liste mutable où ajouter les lignes non matchées
        warnings     : liste mutable où ajouter les warnings (formats KO)

    Returns:
        Nombre de lignes effectivement importées (UPDATE réussi).
    """
    db_path = _classeur_db_path(classeur)
    importees = 0

    # Index global des artworks par set_code (cardinfo.db) — utilisé
    # uniquement pour enrichir le diagnostic des non-trouvées avec rang > 0.
    # Construit avant l'ouverture du classeur pour éviter d'imbriquer
    # deux connexions sqlite simultanées sur des bases différentes.
    artworks_globaux = _build_artworks_globaux_index(classeur)

    with sqlite_ctx(db_path) as conn:
        # Index (set_code, rank, rarity) → [{rowid, name, name_fr}]
        artwork_index = _build_artwork_index(conn)

        # Index dérivés pour le diagnostic enrichi des non-trouvées.
        # On les calcule UNE FOIS ici (et non par ligne CSV) pour garder
        # le diagnostic en O(1) par carte non matchée.
        set_codes_connus  = {sc for (sc, _r, _ra) in artwork_index.keys()}
        sc_rank_connus    = {(sc, r) for (sc, r, _ra) in artwork_index.keys()}
        # Rang max par set_code, pour distinguer "rang invalide" de
        # "rang potentiellement valide globalement mais absent ici".
        rang_max_par_set: dict = defaultdict(int)
        for (sc, rank, _ra) in artwork_index.keys():
            if rank > rang_max_par_set[sc]:
                rang_max_par_set[sc] = rank

        # Pré-traitement des lignes CSV : on agrège les doublons
        # (mêmes set_code/rarity/artwork/edition) en sommant leurs quantités.
        # C'est la seule partie "additive" de l'import — entre lignes du
        # MÊME CSV, pas entre CSV et DB.
        agreges: dict = defaultdict(lambda: {"qty": 0, "qualite": "", "rows_orig": []})

        for row in rows_csv:
            code_csv      = (row.get("Code") or "").strip()
            nom_csv       = (row.get("Nom de la carte") or "").strip()
            rar_code_csv  = (row.get("Rareté") or "").strip()
            art_rank_csv  = _decode_artwork_rank(row.get("N° Artwork"))
            edition_csv, qualite_csv = _decode_edition_et_qualite(row)
            qty_csv       = _parse_quantite(row.get("Quantité"))

            if qty_csv is None or qty_csv == 0:
                # Quantité 0 ou invalide : on n'import rien (Scanflip ne devrait
                # pas écrire de ligne avec qty=0 mais on est défensif).
                if qty_csv is None:
                    warnings.append(
                        f"{classeur} : quantité invalide pour {code_csv} ({row.get('Quantité')!r})"
                    )
                continue

            if not is_known_code(rar_code_csv):
                warnings.append(
                    f"{classeur} : rareté inconnue '{rar_code_csv}' pour {code_csv}"
                )
                continue

            # Conversion CSV → format DB (set_code en EN)
            set_code_local = _convert_set_code_to_local(code_csv, langue_locale="EN")
            rarity_full    = _trouver_rarity_full(rar_code_csv, prefer_lang="en")

            # Matching exact par (set_code, rang_artwork, rarité).
            # Le nom n'intervient PAS dans la clé : il est tolérant aux
            # divergences typographiques entre Scanflip et YGOJSON/YGOPRODeck.
            candidats = artwork_index.get(
                (set_code_local, art_rank_csv, rarity_full), []
            )

            # Fallback 1 : essayer le code CSV brut (cas classeurs legacy
            # qui auraient stocké le set_code en FR au lieu de EN).
            if not candidats and set_code_local != code_csv:
                candidats = artwork_index.get(
                    (code_csv, art_rank_csv, rarity_full), []
                )

            # Fallback 2 — Robustesse aux classeurs legacy avec set_codes
            # mixtes (mai 2026, bug SDWD).
            #
            # Contexte : avant le fix de _build_rows_from_local_db, certains
            # classeurs (typiquement Structure Decks reprints) ont été créés
            # avec un mélange de set_codes EN et FR/DE/IT/ES/PT, à cause
            # d'une jointure trop stricte sur la rareté entre locales.
            # Conséquence : à l'import, ni `SDWD-EN013` (canonique) ni
            # `SDWD-FR013` (CSV brut) ne suffisaient si la BDD stockait
            # par exemple `SDWD-IT013` ou si la rareté en DB divergeait
            # ("Short Print" en EN, "Common" en FR).
            #
            # Ce fallback teste les deux dimensions du problème :
            #   2a. Matching par set_code dans toutes les variantes
            #       linguistiques connues (avec rareté exacte).
            #   2b. Matching par (set_code, rang) seul, sans rareté,
            #       SI une seule entrée existe dans le classeur pour
            #       ce couple — c'est forcément la bonne (pas de risque
            #       d'ambiguïté).
            #
            # Le 2b couvre le cas où le classeur a stocké la rareté
            # autrement que ce que le CSV indique (typique YGOJSON
            # incomplet). On émet un warning informatif pour traçabilité.
            if not candidats:
                # Génère les variantes linguistiques du code CSV
                variantes = _generer_variantes_set_code(code_csv)
                for var in variantes:
                    if var == code_csv or var == set_code_local:
                        continue  # déjà testé
                    candidats = artwork_index.get(
                        (var, art_rank_csv, rarity_full), []
                    )
                    if candidats:
                        break

            if not candidats:
                # Fallback 2b — match par (set_code, rang) sans contrainte
                # rareté, SI une seule entrée match.
                cles_a_tester = [(set_code_local, art_rank_csv)]
                if code_csv != set_code_local:
                    cles_a_tester.append((code_csv, art_rank_csv))
                for var in _generer_variantes_set_code(code_csv):
                    if var not in (set_code_local, code_csv):
                        cles_a_tester.append((var, art_rank_csv))

                for sc_test, rk_test in cles_a_tester:
                    matchs_sans_rarete = []
                    for (idx_sc, idx_rk, idx_ra), entries in artwork_index.items():
                        if idx_sc == sc_test and idx_rk == rk_test:
                            matchs_sans_rarete.extend(entries)
                    # Matching ambigu si > 1 rareté pour ce set_code+rang :
                    # on s'abstient pour ne pas attribuer la mauvaise rareté
                    # à une carte. On préfère la lister en non-trouvée pour
                    # que l'utilisateur fasse un choix conscient (ou corrige
                    # son CSV).
                    if len(matchs_sans_rarete) == 1:
                        candidats = matchs_sans_rarete
                        warnings.append(
                            f"{classeur} : {code_csv} matché sur set_code+rang "
                            f"sans contrainte rareté (CSV='{rar_code_csv}', "
                            f"DB='{matchs_sans_rarete[0].get('rarity_db_str', '?')}'). "
                            "Vérifier la rareté en BDD si import régulier."
                        )
                        break

            if not candidats:
                # Diagnostic enrichi — aide l'utilisateur à comprendre
                # POURQUOI une carte n'a pas été trouvée. Distingue quatre
                # catégories pour ne pas afficher un opaque
                # "carte non trouvée".
                #
                # Le champ "categorie" est machine-lisible (pas localisé) et
                # destiné à l'UI : il permettra au futur lot "Artworks alt
                # à l'import CSV" de filtrer directement les cas
                # "artwork_alt_non_tagge" sans parser la chaîne "raison".
                set_code_pour_diag = (
                    set_code_local if set_code_local in set_codes_connus
                    else code_csv if code_csv in set_codes_connus
                    else None
                )
                # Nb d'artworks globaux connus pour ce set (via cardinfo.db).
                # 0 si cardinfo.db est absent ou ne connaît pas ce set_code.
                nb_arts_globaux = artworks_globaux.get(
                    set_code_pour_diag or set_code_local, 0
                )

                if set_code_pour_diag is None:
                    # 1. Set_code totalement absent du classeur
                    categorie = "set_code_absent"
                    raison = (
                        f"Set_code {code_csv} absent du classeur "
                        "(carte ajoutée au set après création du classeur ?)"
                    )

                elif (set_code_pour_diag, art_rank_csv) not in sc_rank_connus:
                    # 2. Le set_code est connu mais le rang artwork demandé
                    #    n'existe pas dans le classeur. Affiner :
                    #    - si la carte a plusieurs artworks globaux ET le
                    #      rang demandé > 0, c'est probablement un artwork
                    #      alt connu de YGOPRODeck mais non taggé sur ce set
                    #    - sinon, c'est un vrai rang invalide
                    rang_max_classeur = rang_max_par_set.get(set_code_pour_diag, 0)
                    if art_rank_csv > 0 and nb_arts_globaux > rang_max_classeur + 1:
                        categorie = "artwork_alt_non_tagge"
                        raison = (
                            f"Artwork alternatif {art_rank_csv} de {code_csv} "
                            f"connu globalement ({nb_arts_globaux} artworks au total) "
                            "mais non référencé par YGOPRODeck pour ce set. "
                            "Sera importable via l'UI Artworks alt (à venir)."
                        )
                    elif art_rank_csv > 0:
                        categorie = "rang_invalide"
                        raison = (
                            f"Rang artwork {art_rank_csv} hors limites pour {code_csv} "
                            f"(la carte n'a qu'{nb_arts_globaux} artwork(s) connu(s))."
                            if nb_arts_globaux > 0 else
                            f"Rang artwork {art_rank_csv} hors limites pour {code_csv}."
                        )
                    else:
                        # rang = 0 absent : très inhabituel (le rang 0 doit
                        # toujours exister si le set_code est connu). On
                        # met un message générique.
                        categorie = "rang_invalide"
                        raison = (
                            f"Aucun artwork de rang 0 trouvé pour {code_csv} "
                            "dans le classeur."
                        )

                else:
                    # 3. set_code + rang OK, donc c'est la rareté qui manque
                    categorie = "rarete_absente"
                    raison = (
                        f"Rareté '{rar_code_csv}' ({rarity_full}) absente "
                        f"pour {code_csv} dans le classeur."
                    )

                entry_nt = {
                    "classeur":   classeur,
                    "code":       code_csv,
                    "nom":        nom_csv,
                    "rarete":     rar_code_csv,
                    "artwork":    art_rank_csv,
                    "categorie":  categorie,
                    "raison":     raison,
                }
                # Enrichissement pour la catégorie `artwork_alt_non_tagge` :
                # les champs ci-dessous permettent au module
                # `artwork_alt_resolver` de rejouer la ligne CSV (INSERT
                # nouvel artwork + UPDATE qty/qualité/édition) sans avoir
                # à re-parser le CSV ni reconvertir les codes.
                # On ne les ajoute QUE pour cette catégorie afin de ne pas
                # polluer les autres entrées (rang_invalide, rarete_absente,
                # set_code_absent) qui n'en ont pas besoin.
                if categorie == "artwork_alt_non_tagge":
                    entry_nt["set_code_local"] = set_code_local
                    entry_nt["rarete_full"]    = rarity_full
                    entry_nt["qty_csv"]        = qty_csv
                    entry_nt["qualite_csv"]    = qualite_csv
                    entry_nt["edition_csv"]    = edition_csv
                non_trouvees.append(entry_nt)
                continue

            # Si plusieurs candidats matchent — situation théoriquement
            # impossible avec la clé (set_code, rank, rarity) — on prend
            # le premier. Défensif, ne devrait jamais se produire en pratique.
            rowid_cible = candidats[0]["rowid"]

            # Agrégation des doublons CSV vers la même carte DB
            cle_agreg = (rowid_cible, edition_csv)
            agreges[cle_agreg]["qty"] += qty_csv
            agreges[cle_agreg]["qualite"] = qualite_csv  # dernier gagne
            agreges[cle_agreg]["rows_orig"].append(row)

        # Application des UPDATEs en mode REPLACE
        for (rowid, edition), data in agreges.items():
            qty       = data["qty"]
            qualite   = data["qualite"]
            possessed = 1 if qty > 0 else 0
            try:
                conn.execute("""
                    UPDATE cards
                    SET possessed = ?,
                        quantite  = ?,
                        qualite   = ?,
                        edition   = ?
                    WHERE rowid = ?
                """, (possessed, qty, qualite, edition, rowid))
                importees += 1
            except Exception as e:
                warnings.append(
                    f"{classeur} : erreur UPDATE rowid={rowid} ({e})"
                )

    return importees


# ─────────────────────────────────────────────────────────────────────────────
# API publique
# ─────────────────────────────────────────────────────────────────────────────

def importer_csv(chemin_fichier: str,
                 classeur_filtre: str | None = None,
                 creer_classeurs_absents: bool = False,
                 creer_classeur_callback=None) -> dict:
    """
    Importe un CSV au format Scanflip dans la collection.

    Args:
        chemin_fichier   : chemin du fichier CSV à importer.
        classeur_filtre  : si donné, n'importe que les lignes dont
                           Extension == classeur_filtre. Sinon, importe
                           toutes les lignes en les distribuant par classeur.
        creer_classeurs_absents : si True, tente de créer les classeurs
                           présents dans le CSV mais absents en local.
                           Passe par l'orchestrateur `create_classeur()` :
                             - tentative cardinfo.db d'abord (instantané) ;
                             - bascule sur l'API YGOPRODeck si les données
                               locales semblent incomplètes (heuristique
                               raretés/carte) — cohérent avec le bouton
                               "Nouveau classeur" de l'accueil ;
                             - fallback final sur le local si l'API est down,
                               de sorte qu'un import reste possible hors-ligne.
                           Si toutes les sources échouent, l'erreur remonte
                           proprement dans `classeurs_creation_echec`.
        creer_classeur_callback : callable optionnel pour personnaliser la
                           création des classeurs absents. Signature attendue :
                               callback(code_classeur: str) -> (
                                   succes: bool,
                                   raison: str | None,
                               )
                           Le callback peut être bloquant (ex : il pousse
                           dans FileAttenteClasseur et attend la fin de la
                           tâche) — c'est ainsi que l'UI rend les créations
                           visibles dans le centre d'activité, plutôt que
                           d'appeler `create_classeur()` directement (qui
                           est totalement silencieux pour l'utilisateur).
                           Si None ou si `creer_classeurs_absents` est
                           False, on retombe sur l'appel synchrone direct
                           à `create_classeur()` — comportement historique
                           inchangé pour tous les callers existants.

    Returns:
        dict {
            "total_lignes":             int,
            "importees":                int,
            "ignorees":                 int,
            "non_trouvees":             list[dict],  # cf format ci-dessous
            "non_trouvees_total":       int,
            "classeurs_traites":        list,
            "classeurs_inconnus":       list,    # absents même après création
            "classeurs_crees":          list,    # nouvellement créés
            "classeurs_creation_echec": list[dict],  # [{"classeur": ..., "raison": ...}]
            "warnings":                 list,
            "warnings_total":           int,
        }

    Format d'une ligne `non_trouvees` :
        {
            "classeur":  str,    # code du classeur (ex 'RA02')
            "code":      str,    # set_code CSV brut (ex 'RA02-FR001')
            "nom":       str,    # nom CSV de la carte
            "rarete":    str,    # code rareté Scanflip (ex 'SCR')
            "artwork":   int,    # rang artwork (0 = principal)
            "categorie": str,    # cf ci-dessous (machine-lisible)
            "raison":    str,    # explication humaine
        }

    Champs supplémentaires UNIQUEMENT pour la catégorie `artwork_alt_non_tagge`
    (utilisés par module.import_csv.artwork_alt_resolver pour rejouer la
    ligne CSV après que l'utilisateur a choisi un artwork) :
        {
            "set_code_local": str,    # code DB normalisé en EN (ex 'RA02-EN001')
            "rarete_full":    str,    # nom complet rareté DB
            "qty_csv":        int,    # quantité de la ligne CSV
            "qualite_csv":    str,    # M/NM/EX/...
            "edition_csv":    str|None,  # '1st' / 'unlimited' / 'limited' / None
        }

    Catégories possibles (champ "categorie") :
        "set_code_absent"        : le set_code n'existe pas dans le classeur
        "rang_invalide"          : le rang artwork demandé n'existe pas
                                   ET la carte n'a pas plus d'artworks
                                   globalement connus → CSV erroné
        "artwork_alt_non_tagge"  : artwork alt connu globalement mais
                                   YGOPRODeck ne le taggue pas pour ce set.
                                   Traité par module.import_csv.artwork_alt_resolver
                                   + UI artwork_alt_ui (dialog ouvert auto
                                   après l'import).
        "rarete_absente"         : rareté demandée non disponible pour
                                   ce set_code dans le classeur

    Mode REPLACE : la quantité du CSV remplace celle de la DB.
    Si plusieurs lignes du CSV pointent vers la même carte physique,
    leurs quantités sont SOMMÉES avant le replace.
    """
    if not os.path.exists(chemin_fichier):
        raise FileNotFoundError(f"Fichier introuvable : {chemin_fichier}")

    rows_csv = _read_csv_rows(chemin_fichier)
    total_lignes = len(rows_csv)

    # Groupement par classeur (Extension), en filtrant si demandé
    par_classeur: dict[str, list] = defaultdict(list)
    for row in rows_csv:
        ext = (row.get("Extension") or "").strip()
        if not ext:
            continue
        if classeur_filtre and ext != classeur_filtre:
            continue
        par_classeur[ext].append(row)

    non_trouvees: list = []
    warnings: list = []
    classeurs_traites: list = []
    classeurs_inconnus: list = []
    classeurs_crees: list = []
    classeurs_creation_echec: list = []
    importees_total = 0

    # ── PHASE 1 : création des classeurs absents (si demandé) ────────────
    # Si `creer_classeur_callback` est fourni, on l'utilise — ce qui permet
    # à l'UI de rendre les créations visibles via FileAttenteClasseur.
    # Sinon on appelle `create_classeur()` directement (chemin historique,
    # synchrone, silencieux : utile pour les scripts batch ou tests).
    #
    # Dans les deux cas, on passe par l'orchestrateur public
    # `create_classeur()` (et NON par `_create_classeur_from_local`
    # directement) pour bénéficier des mêmes garanties que la création
    # manuelle via "Nouveau classeur" :
    #   - tentative locale d'abord (cardinfo.db, instantané),
    #   - heuristique d'incomplétude (`_rows_locales_semblent_incompletes`)
    #     qui bascule sur YGOPRODeck pour les sets dont les `set_prints`
    #     YGOJSON contiennent des artefacts d'artworks alts non réellement
    #     présents dans le set (cas typique : Legendary Decks, Battle Pack,
    #     Duelist Saga — sets contenant des cartes-icônes type Dieux Égyptiens
    #     avec beaucoup d'artworks alternatifs en BDD globale).
    #   - fallback final sur le local si l'API est down (étape 4 documentée
    #     dans create_classeur), donc on ne perd pas la capacité d'import
    #     hors-ligne.
    # Sans cette indirection, l'import CSV créait des classeurs avec des
    # artworks alts "hallucinés" (cf. bug rapporté mai 2026).
    if creer_classeurs_absents:
        codes_a_creer = [
            code for code in par_classeur.keys()
            if not _classeur_existe(code)
        ]
        if creer_classeur_callback is not None:
            # Mode UI : délègue au callback (typiquement FileAttenteClasseur).
            # Le callback est BLOQUANT : il pousse la tâche dans la file
            # puis attend que la création soit effective (même comportement
            # observable côté importer_csv, mais visible côté UI).
            for code_classeur in codes_a_creer:
                try:
                    succes, raison = creer_classeur_callback(code_classeur)
                except Exception as e:
                    classeurs_creation_echec.append({
                        "classeur": code_classeur,
                        "raison":   f"Callback création : {e}",
                    })
                    continue
                if succes:
                    # Vérification de cohérence : le classeur doit
                    # vraiment exister sur disque avant la phase 2.
                    if _classeur_existe(code_classeur):
                        classeurs_crees.append(code_classeur)
                    else:
                        classeurs_creation_echec.append({
                            "classeur": code_classeur,
                            "raison":   "Callback a renvoyé succès mais le "
                                        "classeur n'est pas sur disque.",
                        })
                else:
                    classeurs_creation_echec.append({
                        "classeur": code_classeur,
                        "raison":   raison or "Échec de création (raison non précisée).",
                    })
        else:
            # Mode synchrone direct (rétrocompatibilité, scripts).
            from module.creation_classeur.creation_classeur_service import (
                create_classeur, SetNotInLocalDB,
            )
            for code_classeur in codes_a_creer:
                try:
                    cree = create_classeur(code_classeur)
                    if cree:
                        classeurs_crees.append(code_classeur)
                except SetNotInLocalDB as e:
                    # Set absent localement ET non rattrapé par l'API
                    # (rare : seulement si create_classeur n'a pas pu basculer)
                    classeurs_creation_echec.append({
                        "classeur": code_classeur,
                        "raison":   str(e),
                    })
                except ValueError as e:
                    # `create_classeur` ré-emballe en ValueError quand local+API
                    # échouent tous les deux (réseau down + set inconnu local).
                    # Le message inclut déjà les deux raisons.
                    classeurs_creation_echec.append({
                        "classeur": code_classeur,
                        "raison":   str(e),
                    })
                except Exception as e:
                    classeurs_creation_echec.append({
                        "classeur": code_classeur,
                        "raison":   f"Erreur inattendue : {e}",
                    })

    # ── PHASE 1.5 : auto-migration des classeurs ciblés ──────────────────
    # Idempotent. Cas d'usage : import dans des classeurs créés avant les
    # fixes d'avril/mai 2026 où certains set_codes étaient en FR/IT (au
    # lieu d'EN canonique) et certaines raretés stockées comme chiffres
    # ("2"/"3" au lieu de "Common"). Sans migration préalable, le matching
    # set_code/rareté du CSV échoue silencieusement et les cartes restent
    # marquées non-possédées alors qu'elles sont bien dans le CSV.
    #
    # Pour les classeurs récents (créés post-fix ou après PHASE 1 ci-dessus),
    # c'est un no-op. Coût négligeable (quelques SELECT + UPDATE bornés).
    try:
        from module.utilitaire.migration_set_codes import reparer_classeur
        for code_classeur in par_classeur.keys():
            if _classeur_existe(code_classeur):
                try:
                    reparer_classeur(code_classeur)
                except Exception as e_inner:
                    warnings.append(
                        f"{code_classeur} : auto-migration ignorée ({e_inner})"
                    )
    except Exception as e:
        warnings.append(f"Auto-migration globale ignorée : {e}")

    # ── PHASE 2 : import dans les classeurs existants ────────────────────
    for code_classeur, rows in par_classeur.items():
        if not _classeur_existe(code_classeur):
            classeurs_inconnus.append(code_classeur)
            continue
        try:
            n = _importer_dans_classeur(
                code_classeur, rows, non_trouvees, warnings
            )
            if n > 0:
                classeurs_traites.append(code_classeur)
                importees_total += n
        except Exception as e:
            warnings.append(f"{code_classeur} : erreur globale ({e})")

    ignorees = max(0, total_lignes - importees_total)

    return {
        "total_lignes":             total_lignes,
        "importees":                importees_total,
        "ignorees":                 ignorees,
        "non_trouvees":             non_trouvees[:NON_TROUVEES_PREVIEW_MAX],
        "non_trouvees_total":       len(non_trouvees),
        "classeurs_traites":        sorted(classeurs_traites),
        "classeurs_inconnus":       sorted(classeurs_inconnus),
        "classeurs_crees":          sorted(classeurs_crees),
        "classeurs_creation_echec": classeurs_creation_echec,
        "warnings":                 warnings[:WARNINGS_PREVIEW_MAX],
        "warnings_total":           len(warnings),
    }


def detecter_classeurs_absents(chemin_fichier: str) -> list[str]:
    """
    Helper utilisé par l'UI avant l'import : retourne la liste des
    classeurs présents dans le CSV mais absents en local.

    Permet d'afficher la dialog de confirmation utilisateur AVANT de
    lancer importer_csv() avec creer_classeurs_absents=True.

    Retourne [] si toutes les extensions du CSV existent déjà localement.
    """
    if not os.path.exists(chemin_fichier):
        return []
    try:
        rows_csv = _read_csv_rows(chemin_fichier)
    except Exception:
        return []
    extensions = set()
    for row in rows_csv:
        ext = (row.get("Extension") or "").strip()
        if ext:
            extensions.add(ext)
    return sorted([ext for ext in extensions if not _classeur_existe(ext)])
