"""
artwork_alt_resolver.py — Résolution des artworks alternatifs ignorés à l'import CSV.

CONTEXTE
────────
À l'import CSV (cf import_collection.py), une ligne avec `N° Artwork = "1"`
peut tomber en non-trouvée avec la catégorie `artwork_alt_non_tagge` :
  - cardinfo.db connaît globalement plusieurs artworks pour la carte
  - YGOPRODeck ne tagge aucun de ces artworks alt sur le set demandé
  - le classeur local n'a donc qu'un seul rang (0) pour ce set_code

Ce module permet à l'utilisateur de choisir manuellement, parmi les
artworks alt connus globalement, lesquels correspondent à ses cartes
physiques, puis insère les lignes manquantes dans le classeur SQLite.

RAPPEL
──────
Cette fonctionnalité est une rustine temporaire destinée aux bases
existantes. La nouvelle version de l'application gérera nativement les
artworks alt à la création du classeur — `artwork_alt_non_tagge` ne
devrait plus se produire avec une base neuve.

CONTRAT
───────
Le module ne touche PAS au module `anomalie/`. La logique d'INSERT est
dupliquée localement (5 lignes de SQL) à dessein, plutôt que d'importer
`corriger_anomalie` — cf. décision projet (le module anomalie reste
intouché).

Le module ne lance JAMAIS un scan d'anomalies global. Toutes les requêtes
sont limitées aux set_codes effectivement présents dans les non-trouvées
du CSV importé.
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
from collections import defaultdict
from module.centralisation_dossier import (
    CLASSEUR_FOLDER, CARDINFO_DB, sqlite_ctx,
)
from module.logger_app import log


# ─────────────────────────────────────────────────────────────────────────────
# Listing des propositions
# ─────────────────────────────────────────────────────────────────────────────

def lister_propositions_artwork_alt(non_trouvees: list[dict]) -> list[dict]:
    """
    Pour chaque non-trouvée de catégorie `artwork_alt_non_tagge`, requête
    cardinfo.db et le classeur local pour proposer à l'utilisateur les
    artworks alt connus mais non taggés sur ce set.

    Args:
        non_trouvees : liste retournée par `importer_csv()` dans le champ
                       `non_trouvees`. Les dicts attendus pour cette catégorie
                       doivent contenir (en plus des champs standard) :
                          - set_code_local  (code DB, ex 'RA02-EN001')
                          - rarete_full     (nom complet rareté DB)
                          - qty_csv, qualite_csv, edition_csv

    Returns:
        Liste de dicts groupés par (classeur, set_code_local, name_db).
        Chaque entrée contient :
          - classeur          : code du classeur
          - code_csv          : code original CSV (ex 'RA02-FR001')
          - set_code_local    : code DB (ex 'RA02-EN001')
          - name              : nom EN tel que stocké dans le classeur
          - name_fr           : nom FR (si disponible)
          - lignes_csv        : liste des (rarete_code, rarete_full, qty_csv,
                                qualite_csv, edition_csv) à appliquer
          - propositions      : liste d'artworks alt proposés
                                [{card_image_uuid, card_image_id,
                                  card_image_url, card_image_small}, ...]
          - artworks_existants: liste des artworks DÉJÀ présents dans le
                                classeur pour ce set_code (info pour l'UI)

        Liste vide si :
          - aucune non-trouvée n'a la catégorie ciblée
          - cardinfo.db inaccessible
          - aucun classeur des non-trouvées n'existe localement
    """
    cibles = [
        nt for nt in (non_trouvees or [])
        if nt.get("categorie") == "artwork_alt_non_tagge"
    ]
    if not cibles:
        return []
    if not os.path.isfile(CARDINFO_DB):
        return []

    # ── Étape 1 : groupement par (classeur, set_code_local) ──────────────────
    # Plusieurs lignes CSV peuvent partager la même carte physique avec des
    # raretés différentes (ex: RA02-FR001 a 7 raretés × artwork=1). On les
    # rassemble pour ne demander qu'UN choix d'artwork par carte.
    groupes: dict = defaultdict(lambda: {
        "lignes_csv": [],
        "code_csv":   "",  # premier code CSV rencontré (peut être FR)
    })
    for nt in cibles:
        classeur = nt.get("classeur") or ""
        set_code_local = nt.get("set_code_local") or ""
        if not classeur or not set_code_local:
            continue
        cle = (classeur, set_code_local)
        groupes[cle]["code_csv"] = nt.get("code", "") or groupes[cle]["code_csv"]
        groupes[cle]["lignes_csv"].append({
            "rarete_code": nt.get("rarete", ""),
            "rarete_full": nt.get("rarete_full", ""),
            "qty_csv":     nt.get("qty_csv", 0),
            "qualite_csv": nt.get("qualite_csv", ""),
            "edition_csv": nt.get("edition_csv"),
            # On conserve le rang artwork du CSV pour info UI ; il n'est
            # PAS utilisé pour le matching (l'utilisateur choisira visuellement).
            "art_rank_csv": nt.get("artwork", 0),
        })

    if not groupes:
        return []

    # ── Étape 2 : lookup cardinfo.db en bloc ─────────────────────────────────
    # Pour chaque set_code unique, on cherche le card_uuid associé puis tous
    # ses artworks (incluant ceux non taggés sur ce set précis).
    set_codes_uniques = sorted({sc for (_cl, sc) in groupes.keys()})
    artworks_par_set = _lookup_artworks_globaux(set_codes_uniques)

    # ── Étape 3 : pour chaque groupe, lister artworks classeur + filtrer ─────
    resultats: list[dict] = []
    for (classeur, set_code_local), data in groupes.items():
        infos_globales = artworks_par_set.get(set_code_local)
        if not infos_globales:
            # Pas d'artworks trouvés dans cardinfo.db → skip
            continue

        existants = _lister_artworks_classeur(classeur, set_code_local)
        ids_existants = {a["card_image_id"] for a in existants if a["card_image_id"]}

        propositions = [
            art for art in infos_globales["artworks"]
            if art["card_image_id"] not in ids_existants
        ]
        if not propositions:
            # Aucun artwork à proposer (tout est déjà dans le classeur).
            # Cas très rare mais défensif : si la condition de diagnostic
            # `artwork_alt_non_tagge` a été levée à tort, on ne fait rien.
            continue

        resultats.append({
            "classeur":          classeur,
            "code_csv":          data["code_csv"],
            "set_code_local":    set_code_local,
            "name":              infos_globales["name"],
            "name_fr":           infos_globales["name_fr"],
            "lignes_csv":        data["lignes_csv"],
            "propositions":      propositions,
            "artworks_existants": existants,
        })

    return resultats


def lister_propositions_pour_carte(
    classeur: str,
    set_code: str,
    rarete_full: str,
    name: str = "",
) -> dict | None:
    """Variante single-card de lister_propositions_artwork_alt.

    Construit un groupe de proposition prêt à passer à
    `afficher_dialog_artwork_alt` pour UNE carte précise du classeur.
    Cas d'usage : clic-droit "Modifier l'artwork" sur une vignette du
    visualiseur classeur. Permet à l'utilisateur d'ajouter manuellement
    un artwork alternatif connu dans cardinfo.db mais pas encore présent
    dans le classeur pour ce slot (set_code+rareté).

    Algorithme aligné sur celui du scanner d'anomalies
    (`module.anomalie.anomalie_service.scanner_et_stocker_anomalies`) :
    on regroupe par NOM de carte plutôt que par card_uuid. Cela évite un
    bug observé sur les cartes très réimprimées (Blue-Eyes White Dragon,
    Dark Magician...) où YGOJSON traite certaines variantes d'artwork
    comme des card_uuid distincts alors qu'elles partagent le même nom.
    Une recherche par card_uuid ne ramassait alors qu'une fraction des
    artworks connus, faisant croire à tort qu'aucun n'était disponible.

    Réutilise les mêmes lookups que la version CSV pour la liste des
    existants en classeur : `_lister_artworks_classeur`. Filtre les
    doublons sur card_image_id ET card_image_uuid pour être robuste aux
    cas où l'un ou l'autre est NULL.

    Args:
        classeur     : code du classeur (ex 'SDWD')
        set_code     : ex 'SDWD-EN001' — DOIT être un code EN canonique
                       présent dans cardinfo.db
        rarete_full  : nom complet de la rareté du slot ciblé (ex 'Common')
        name         : nom EN de la carte (résolu depuis cardinfo.db si vide)

    Returns:
        dict au format `lister_propositions_artwork_alt[i]`, ou None si :
          - cardinfo.db inaccessible
          - set_code introuvable dans cardinfo.db
          - aucune carte connue avec ce nom
          - aucun artwork alt à proposer (tous déjà dans le classeur)
    """
    if not classeur or not set_code:
        return None
    if not os.path.isfile(CARDINFO_DB):
        return None

    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()

            # ── Étape 1 : résoudre le nom EN canonique ───────────────────
            # Stratégie en 2 temps avec fallback :
            #   (a) priorité : résoudre depuis set_code via set_prints+card_texts
            #       → fonctionne quand cardinfo.db connaît le set
            #   (b) fallback : utiliser le `name` passé par l'appelant
            #       → indispensable pour les classeurs créés via l'API
            #       YGOPRODeck dont les set_codes (ex: SDWD-EN001) ne sont
            #       PAS référencés dans set_prints de cardinfo.db
            #
            # Sans cette étape (b), tous les classeurs API + sets non
            # référencés YGOJSON tomberaient en "Aucun artwork alternatif"
            # alors que le nom de la carte est parfaitement disponible et
            # suffit à interroger card_texts → card_images.
            cursor.execute("""
                SELECT ct.name
                FROM set_prints sp
                JOIN card_texts ct ON ct.card_uuid = sp.card_uuid
                                  AND ct.language = 'en'
                WHERE sp.set_code = ?
                  AND sp.card_uuid IS NOT NULL
                LIMIT 1
            """, (set_code,))
            row = cursor.fetchone()
            if row and row[0]:
                name_en = row[0]
            elif name:
                # Fallback : on prend le nom passé par l'appelant.
                # C'est presque toujours le name EN du classeur, mais
                # ça pourrait être en FR selon la préférence d'affichage.
                # On gère les deux dans l'étape 2 ci-dessous.
                name_en = name.strip()
            else:
                # Ni set_code ni name → impossible de continuer.
                return None

            # ── Étape 2 : tous les card_uuid partageant ce nom ───────────
            # Tentative #1 : nom EN (cas standard, le plus sûr car les
            # noms anglais Konami sont quasi uniques).
            cursor.execute("""
                SELECT DISTINCT card_uuid FROM card_texts
                WHERE language = 'en' AND name = ?
                  AND card_uuid IS NOT NULL
            """, (name_en,))
            card_uuids = [r[0] for r in cursor.fetchall() if r[0]]

            # Tentative #2 : si aucun match EN, on élargit à toutes les
            # langues — utile quand l'appelant nous a passé un nom FR
            # (mode d'affichage FR), ou si la BDD a un édge case sur la
            # casse / accents.
            if not card_uuids:
                cursor.execute("""
                    SELECT DISTINCT card_uuid FROM card_texts
                    WHERE name = ? AND card_uuid IS NOT NULL
                """, (name_en,))
                card_uuids = [r[0] for r in cursor.fetchall() if r[0]]

            if not card_uuids:
                return None

            # ── Étape 3 : tous les card_images de tous ces card_uuid ─────
            ph = ",".join("?" * len(card_uuids))
            cursor.execute(f"""
                SELECT uuid, card_uuid, ygoprodeck_image_id, card_url, art_url
                FROM card_images
                WHERE card_uuid IN ({ph})
            """, card_uuids)
            arts_brut = cursor.fetchall()

            # ── Étape 4 : récupère les noms canoniques EN et FR ──────────
            cursor.execute(f"""
                SELECT name FROM card_texts
                WHERE language='en' AND card_uuid IN ({ph}) LIMIT 1
            """, card_uuids)
            r_en = cursor.fetchone()
            name_en_canonical = r_en[0] if r_en and r_en[0] else name_en

            cursor.execute(f"""
                SELECT name FROM card_texts
                WHERE language = 'fr' AND card_uuid IN ({ph})
                LIMIT 1
            """, card_uuids)
            row_fr = cursor.fetchone()
            name_fr = row_fr[0] if row_fr and row_fr[0] else ""
    except Exception as e:
        log.warning(f"lister_propositions_pour_carte lookup : {e}")
        return None

    # ── Étape 5 : déduplication + tri stable par image_id ───────────────
    seen_uuids: set = set()
    arts_uniques: list[dict] = []
    for img_uuid, _cuid, img_id, card_url, art_url in sorted(
        arts_brut,
        key=lambda r: (r[2] or 0, r[0] or ""),  # tri par (image_id, uuid)
    ):
        if img_uuid in seen_uuids:
            continue
        seen_uuids.add(img_uuid)
        arts_uniques.append({
            "card_image_uuid":  img_uuid,
            "card_image_id":    img_id,
            "card_image_url":   card_url or "",
            "card_image_small": art_url or "",
        })

    if not arts_uniques:
        return None

    # ── Étape 6 : filtrer ceux déjà dans le classeur ────────────────────
    # Filtre sur image_id ET image_uuid — robuste au cas où l'un est NULL
    # (classeur créé via API sans uuid, ou cardinfo.db sans ygoprodeck_image_id).
    existants = _lister_artworks_classeur(classeur, set_code)
    ids_existants   = {a["card_image_id"]   for a in existants if a["card_image_id"]}
    uuids_existants = {a["card_image_uuid"] for a in existants if a["card_image_uuid"]}

    propositions = [
        art for art in arts_uniques
        if (art["card_image_id"]   not in ids_existants
            and art["card_image_uuid"] not in uuids_existants)
    ]
    if not propositions:
        return None

    return {
        "classeur":          classeur,
        "code_csv":          set_code,
        "set_code_local":    set_code,
        "name":              name_en_canonical,
        "name_fr":           name_fr,
        "lignes_csv":        [{
            "rarete_code":  "",
            "rarete_full":  rarete_full or "",
            "qty_csv":      0,
            "qualite_csv": "",
            "edition_csv":  None,
            "art_rank_csv": 0,
        }],
        "propositions":      propositions,
        "artworks_existants": existants,
    }


def _lookup_artworks_globaux(set_codes: list[str]) -> dict:
    """
    Pour une liste de set_codes (ex ['RA02-EN001', 'LDK2-ENK01']), retourne
    pour chacun :
      - le card_uuid (un seul attendu par set_code dans 99% des cas)
      - le name EN (depuis card_texts.language='en')
      - le name FR (depuis card_texts.language='fr', '' si absent)
      - la liste TOUS les artworks de cette carte connus dans card_images

    Returns:
        dict { set_code → {
            card_uuid: str,
            name: str,
            name_fr: str,
            artworks: list[{card_image_uuid, card_image_id,
                            card_image_url, card_image_small}],
        }}

    En cas d'erreur d'accès cardinfo.db, retourne dict vide. Le diagnostic
    dégrade gracieusement (l'utilisateur verra "aucune proposition").
    """
    if not set_codes:
        return {}

    result: dict = {}
    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            cursor = conn.cursor()
            CHUNK = 500
            for i in range(0, len(set_codes), CHUNK):
                batch = set_codes[i:i + CHUNK]
                placeholders = ",".join("?" * len(batch))

                # Étape 1 : set_code → card_uuid (premier rencontré).
                # On agrège les card_uuid distincts par set_code en gardant
                # uniquement le premier (les rares doublons sont ignorés).
                cursor.execute(f"""
                    SELECT sp.set_code, sp.card_uuid
                    FROM set_prints sp
                    WHERE sp.set_code IN ({placeholders})
                      AND sp.card_uuid IS NOT NULL
                """, batch)
                set_to_uuid: dict = {}
                for sc, uuid in cursor.fetchall():
                    if sc and uuid and sc not in set_to_uuid:
                        set_to_uuid[sc] = uuid

                if not set_to_uuid:
                    continue

                uuids_uniques = list(set(set_to_uuid.values()))
                ph_uuids = ",".join("?" * len(uuids_uniques))

                # Étape 2 : card_uuid → noms EN/FR
                cursor.execute(f"""
                    SELECT card_uuid, language, name
                    FROM card_texts
                    WHERE card_uuid IN ({ph_uuids})
                      AND language IN ('en', 'fr')
                """, uuids_uniques)
                noms: dict = defaultdict(lambda: {"en": "", "fr": ""})
                for uuid, lang, name in cursor.fetchall():
                    if uuid and name:
                        noms[uuid][lang] = name

                # Étape 3 : card_uuid → liste artworks
                cursor.execute(f"""
                    SELECT uuid, card_uuid, ygoprodeck_image_id, card_url, art_url
                    FROM card_images
                    WHERE card_uuid IN ({ph_uuids})
                """, uuids_uniques)
                arts_par_uuid: dict = defaultdict(list)
                for img_uuid, card_uuid, img_id, card_url, art_url in cursor.fetchall():
                    arts_par_uuid[card_uuid].append({
                        "card_image_uuid":  img_uuid,
                        "card_image_id":    img_id,
                        "card_image_url":   card_url or "",
                        "card_image_small": art_url or "",
                    })

                # Étape 4 : assemblage par set_code
                for set_code, card_uuid in set_to_uuid.items():
                    arts = arts_par_uuid.get(card_uuid, [])
                    if not arts:
                        continue
                    # Tri stable par card_image_id (même ordre que rang dans
                    # le classeur) pour que les propositions soient affichées
                    # de manière cohérente avec les rangs locaux.
                    arts_tries = sorted(
                        arts,
                        key=lambda a: (a["card_image_id"] or 0, a["card_image_uuid"] or ""),
                    )
                    n = noms.get(card_uuid, {})
                    result[set_code] = {
                        "card_uuid": card_uuid,
                        "name":      n.get("en", "") if n else "",
                        "name_fr":   n.get("fr", "") if n else "",
                        "artworks":  arts_tries,
                    }
    except Exception as e:
        log.warning(f"_lookup_artworks_globaux : {e}")
        return {}

    return result


def _lister_artworks_classeur(classeur: str, set_code: str) -> list[dict]:
    """
    Liste les artworks DÉJÀ présents dans le classeur SQLite pour un set_code.

    Returns:
        Liste de dicts {rowid, card_image_id, card_image_uuid, rarity, name,
        name_fr, sort_order}. Vide si classeur inexistant.

    Utilisé pour :
      - filtrer les propositions (ne pas re-proposer un artwork déjà importé)
      - servir de "ligne source" lors de l'INSERT (cf. appliquer_choix_artworks)
    """
    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.isfile(db_path):
        return []

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()
            cursor.execute("PRAGMA table_info(cards)")
            cols = {c[1] for c in cursor.fetchall()}
            has_name_fr = "name_fr" in cols

            if has_name_fr:
                cursor.execute("""
                    SELECT rowid, card_image_id, card_image_uuid, rarity,
                           name, name_fr, sort_order
                    FROM cards
                    WHERE set_code = ?
                """, (set_code,))
            else:
                cursor.execute("""
                    SELECT rowid, card_image_id, card_image_uuid, rarity,
                           name, '' AS name_fr, sort_order
                    FROM cards
                    WHERE set_code = ?
                """, (set_code,))
            rows = cursor.fetchall()
    except Exception as e:
        log.warning(f"_lister_artworks_classeur({classeur}, {set_code}) : {e}")
        return []

    return [
        {
            "rowid":           r[0],
            "card_image_id":   r[1],
            "card_image_uuid": r[2],
            "rarity":          r[3] or "",
            "name":            r[4] or "",
            "name_fr":         r[5] or "",
            "sort_order":      r[6] or 0,
        }
        for r in rows
    ]


# ─────────────────────────────────────────────────────────────────────────────
# Application des choix utilisateur
# ─────────────────────────────────────────────────────────────────────────────

def appliquer_choix_artworks(decisions: list[dict]) -> dict:
    """
    Pour chaque décision validée par l'utilisateur, INSERT la nouvelle
    ligne d'artwork dans le classeur SQLite + UPDATE possessed/quantite/
    qualite/edition selon les valeurs CSV.

    Args:
        decisions : liste de dicts au format :
            {
                "classeur":         str,    # code du classeur (ex 'RA02')
                "set_code_local":   str,    # ex 'RA02-EN001'
                "rarete_full":      str,    # ex 'Secret Rare'
                # Artwork choisi (depuis propositions[].propositions) :
                "card_image_uuid":  str,
                "card_image_id":    int,
                "card_image_url":   str,
                "card_image_small": str,
                # Données CSV à appliquer après INSERT :
                "qty_csv":          int,
                "qualite_csv":      str,
                "edition_csv":      str | None,
            }

    Returns:
        dict {
            "appliquees":         int,
            "echecs":             list[dict],   # [{"decision": d, "raison": str}]
            "classeurs_touches":  list[str],    # set ordonné, à passer à FileAttenteClasseur
        }

    Politique :
      - Un échec sur une décision ne bloque pas les autres.
      - Si plusieurs décisions ciblent la même carte physique (set_code +
        rareté + même artwork), seule la première est appliquée — les
        suivantes sont marquées "déjà appliquée" (pas un échec).
      - L'INSERT recopie les métadonnées d'une ligne existante du classeur
        (sort_order, card_type, atk/def/level, attribute, race, set_name,
        rarity_code) — logique alignée sur `corriger_anomalie` mais
        dupliquée localement (le module anomalie reste intouché).
    """
    appliquees = 0
    echecs: list = []
    classeurs_touches: set = set()

    # Index pour détecter les doublons de décision dans le même appel.
    # Clé = (classeur, set_code, rarete_full, card_image_id).
    deja_traites: set = set()

    for decision in (decisions or []):
        try:
            classeur     = (decision.get("classeur") or "").strip()
            set_code     = (decision.get("set_code_local") or "").strip()
            rarete_full  = (decision.get("rarete_full") or "").strip()
            new_uuid     = (decision.get("card_image_uuid") or "").strip()
            new_id       = decision.get("card_image_id")
            new_url      = (decision.get("card_image_url") or "").strip()
            new_small    = (decision.get("card_image_small") or "").strip()

            qty          = int(decision.get("qty_csv") or 0)
            qualite      = decision.get("qualite_csv") or ""
            edition      = decision.get("edition_csv")

            if not classeur or not set_code or not rarete_full:
                echecs.append({
                    "decision": decision,
                    "raison":   "Décision incomplète (classeur/set_code/rareté manquant).",
                })
                continue
            if not new_id and not new_url:
                echecs.append({
                    "decision": decision,
                    "raison":   "Aucune source d'artwork (card_image_id ni URL).",
                })
                continue

            cle_dedupe = (classeur, set_code, rarete_full, new_id)
            if cle_dedupe in deja_traites:
                # Double sélection identique dans le même appel → on
                # n'INSERT qu'une fois, mais on ne signale pas d'erreur.
                continue
            deja_traites.add(cle_dedupe)

            n_inserees, rowid_nouveau = _inserer_artwork_dans_classeur(
                classeur=classeur,
                set_code=set_code,
                rarete_full=rarete_full,
                new_image_uuid=new_uuid,
                new_image_id=new_id,
                new_image_url=new_url,
                new_image_small=new_small,
            )
            if n_inserees == 0 or rowid_nouveau is None:
                echecs.append({
                    "decision": decision,
                    "raison":   "INSERT impossible (ligne source introuvable ou doublon).",
                })
                continue

            # Application qty/qualité/édition sur la nouvelle ligne
            ok = _appliquer_donnees_csv(
                classeur=classeur,
                rowid=rowid_nouveau,
                qty=qty,
                qualite=qualite,
                edition=edition,
            )
            if not ok:
                echecs.append({
                    "decision": decision,
                    "raison":   "INSERT réussi mais UPDATE qty/qualité a échoué.",
                })
                # On compte quand même l'INSERT comme appliqué.

            appliquees += 1
            classeurs_touches.add(classeur)

        except Exception as e:
            echecs.append({
                "decision": decision,
                "raison":   f"Exception : {e}",
            })

    return {
        "appliquees":        appliquees,
        "echecs":            echecs,
        "classeurs_touches": sorted(classeurs_touches),
    }


def remplacer_artwork_carte(
    classeur: str,
    rowid: int,
    new_image_uuid: str,
    new_image_id,
    new_image_url: str,
    new_image_small: str,
) -> bool:
    """
    Remplace EN PLACE l'artwork de la carte identifiée par son `rowid`.

    Contrairement à `_inserer_artwork_dans_classeur` (qui crée une nouvelle
    ligne), cette fonction met à jour les champs image de la ligne existante
    et CONSERVE possession / quantité / qualité / édition.

    Anti-doublon : si une AUTRE ligne du classeur porte déjà cet artwork
    (même set_code + rareté + card_image_id), on abandonne pour ne pas créer
    de doublon. Retourne True si le remplacement a réussi.
    """
    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.isfile(db_path):
        return False

    try:
        with sqlite_ctx(db_path) as conn:
            cursor = conn.cursor()

            # Ligne cible
            cursor.execute(
                "SELECT set_code, rarity FROM cards WHERE rowid=?", (rowid,)
            )
            cible = cursor.fetchone()
            if not cible:
                log.warning(f"remplacer_artwork_carte : rowid {rowid} introuvable.")
                return False
            set_code, rarity = cible

            # Anti-doublon : artwork déjà présent sur une autre ligne ?
            if new_image_id is not None:
                cursor.execute("""
                    SELECT rowid FROM cards
                    WHERE set_code=? AND rarity=? AND card_image_id=? AND rowid<>?
                    LIMIT 1
                """, (set_code, rarity, new_image_id, rowid))
                if cursor.fetchone():
                    log.warning(
                        "remplacer_artwork_carte : cet artwork est déjà présent "
                        "sur une autre carte du classeur — remplacement annulé."
                    )
                    return False

            # UPDATE en place des seuls champs image. possessed/quantite/
            # qualite/edition restent inchangés. On conserve l'ancienne
            # valeur d'URL/small si la proposition n'en fournit pas (NULLIF).
            cursor.execute("""
                UPDATE cards
                SET card_image_uuid  = COALESCE(NULLIF(?, ''), card_image_uuid),
                    card_image_id    = ?,
                    card_image_url   = COALESCE(NULLIF(?, ''), card_image_url),
                    card_image_small = COALESCE(NULLIF(?, ''), card_image_small)
                WHERE rowid = ?
            """, (
                new_image_uuid or "", new_image_id,
                new_image_url or "", new_image_small or "",
                rowid,
            ))
        return True
    except Exception as e:
        log.warning(f"remplacer_artwork_carte (rowid={rowid}) : {e}")
        return False


def _inserer_artwork_dans_classeur(
    classeur: str,
    set_code: str,
    rarete_full: str,
    new_image_uuid: str,
    new_image_id,
    new_image_url: str,
    new_image_small: str,
) -> tuple[int, int | None]:
    """
    Insère une nouvelle ligne dans `cards` du classeur, en recopiant les
    métadonnées d'une ligne existante avec même (set_code, rarity).

    Retourne (nb_inserees, rowid_nouveau_ou_None).

    NOTE : logique alignée sur `module.anomalie.anomalie_service.corriger_anomalie`
    mais dupliquée ici à dessein — le module anomalie reste intouché par
    cette fonctionnalité (cf. décision projet).
    """
    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.isfile(db_path):
        return 0, None

    with sqlite_ctx(db_path) as conn:
        cursor = conn.cursor()

        # Anti-doublon : artwork déjà présent pour (set_code, rarity) ?
        if new_image_id is not None:
            cursor.execute("""
                SELECT rowid FROM cards
                WHERE set_code=? AND rarity=? AND card_image_id=?
                LIMIT 1
            """, (set_code, rarete_full, new_image_id))
            if cursor.fetchone():
                return 0, None
        elif new_image_url:
            cursor.execute("""
                SELECT rowid FROM cards
                WHERE set_code=? AND rarity=? AND card_image_url=?
                LIMIT 1
            """, (set_code, rarete_full, new_image_url))
            if cursor.fetchone():
                return 0, None
        else:
            return 0, None

        # Ligne source : on prend une ligne existante du même set_code +
        # même rareté pour copier les métadonnées (name, name_fr, set_name,
        # rarity_code, sort_order, stats…). Si aucune ligne pour cette
        # rareté précise n'existe, on retombe sur n'importe quelle ligne
        # du même set_code (raretés multiples partagent name/stats/etc.).
        cursor.execute("""
            SELECT card_uuid, card_image_uuid, set_code, rarity, rarity_code,
                   set_name, name, name_fr, card_image_url, card_image_small,
                   card_image_id, sort_order,
                   card_type, atk, def_val, level, attribute, race
            FROM cards
            WHERE set_code=? AND rarity=?
            ORDER BY rowid LIMIT 1
        """, (set_code, rarete_full))
        source = cursor.fetchone()
        if not source:
            cursor.execute("""
                SELECT card_uuid, card_image_uuid, set_code, rarity, rarity_code,
                       set_name, name, name_fr, card_image_url, card_image_small,
                       card_image_id, sort_order,
                       card_type, atk, def_val, level, attribute, race
                FROM cards
                WHERE set_code=?
                ORDER BY rowid LIMIT 1
            """, (set_code,))
            source = cursor.fetchone()
        if not source:
            return 0, None

        (s_card_uuid, s_card_image_uuid, s_set_code, s_rarity, s_rarity_code,
         s_set_name, s_name, s_name_fr, s_image_url, s_image_small,
         s_image_id, s_sort_order,
         s_card_type, s_atk, s_def_val, s_level, s_attribute, s_race) = source

        # Si la ligne source venait d'une rareté différente (cas de fallback
        # ci-dessus), on FORCE rarity = rarete_full pour la nouvelle ligne.
        # Le rarity_code peut rester celui de la source si même set_code
        # (cohérent en pratique).
        rarity_finale = rarete_full

        final_url   = new_image_url   or s_image_url
        final_id    = new_image_id    if new_image_id is not None else s_image_id
        final_small = new_image_small or s_image_small
        final_uuid  = new_image_uuid  or s_card_image_uuid

        cursor.execute("""
            INSERT INTO cards
              (card_uuid, card_image_uuid, set_code, rarity, rarity_code,
               set_name, name, name_fr,
               card_image_url, card_image_small, card_image_id,
               sort_order,
               card_type, atk, def_val, level, attribute, race,
               possessed, quantite, qualite, is_custom)
            VALUES (?,?,?,?,?, ?,?,?, ?,?,?, ?, ?,?,?,?,?,?, 0,0,NULL,0)
        """, (
            s_card_uuid,
            final_uuid,
            s_set_code, rarity_finale, s_rarity_code,
            s_set_name, s_name, s_name_fr,
            final_url, final_small, final_id,
            s_sort_order,
            s_card_type, s_atk, s_def_val, s_level, s_attribute, s_race,
        ))
        rowid_nouveau = cursor.lastrowid
        return cursor.rowcount, rowid_nouveau


def _appliquer_donnees_csv(
    classeur: str,
    rowid: int,
    qty: int,
    qualite: str,
    edition,
) -> bool:
    """
    UPDATE possessed/quantite/qualite/edition sur la ligne fraîchement
    insérée. Aligné sur la logique de `_importer_dans_classeur` :
      possessed = 1 si qty>0 sinon 0.
    """
    db_path = os.path.join(CLASSEUR_FOLDER, classeur, f"{classeur}.db")
    if not os.path.isfile(db_path):
        return False

    qty       = max(0, int(qty or 0))
    possessed = 1 if qty > 0 else 0
    qualite   = qualite or ""

    try:
        with sqlite_ctx(db_path) as conn:
            conn.execute("""
                UPDATE cards
                SET possessed = ?,
                    quantite  = ?,
                    qualite   = ?,
                    edition   = ?
                WHERE rowid = ?
            """, (possessed, qty, qualite, edition, rowid))
        return True
    except Exception as e:
        log.warning(f"_appliquer_donnees_csv (rowid={rowid}) : {e}")
        return False
