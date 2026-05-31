# -*- coding: utf-8 -*-
"""
overframe_enrichment.py — Complétion des prints "extended art" (Overframe) via Yugipedia.

PROBLEME RESOLU
  La source YGOJSON (cardinfo.db) est INCOMPLETE pour les sets a traitement
  Overframe : pour une carte chase comme LOCR-JP001 elle ne liste que les prints
  de base (Secret / Prismatic Secret) au lieu des 5 attendus. Manquent les
  variantes Overframe : Ultra Rare *extended art*, Prismatic Secret Rare
  *extended art* et Grand Master Rare. YGOJSON ne distingue pas non plus le
  cadre (une meme rarete peut exister en cadre normal ET en Overframe). Comme
  l'application place les cartes sequentiellement, tout print manquant DECALE
  la grille.

POINT IMPORTANT — L'OVERFRAME N'EST PAS GLOBAL
  Dans LOCH et LOCR, l'extended art (Overframe) n'existe QUE sur un sous-ensemble
  limite de cartes "chase" (cf. Yugipedia LOCH : "18 Prismatic Secret Rares are
  only available with an extended art, and these extended art cards are also
  available as Ultra Rare and Grand Master Rare"). La grande majorite des cartes
  n'a AUCUN print Overframe. La detection ne doit donc jamais marquer une carte
  en extended art « par defaut ».

STRATEGIE — ADDITIVE + CORRECTION DE CADRE (et non remplacement total)
  Yugipedia liste, pour chaque carte chase, une entree « extended art » dont les
  raretes sont les variantes Overframe. On en deduit l'ensemble des prints
  Overframe {(numero, rarete)}, puis on RECONCILIE cardinfo.db :
    - print Overframe deja present (extended_art=1) -> rien a faire ;
    - rarete presente dans les DEUX cadres (ex Prismatic Secret Rare des cartes
      chase) -> on AJOUTE une ligne extended_art=1 en conservant la normale ;
    - rarete Overframe-only (ex Ultra Rare / Grand Master Rare des chase) que
      YGOJSON a importee a tort en cadre normal -> on CORRIGE le flag
      (extended_art 0 -> 1) sans creer de doublon ;
    - sinon -> on AJOUTE la ligne extended_art=1.
  Les metadonnees (card_uuid, card_image_uuid, set_uuid, set_locale_id, edition,
  qty, print_image_url) sont HERITEES d'un print existant du meme numero (tous
  les prints d'une meme carte partagent les memes references de carte).

  Cette approche ne SUPPRIME jamais de print existant : aucune perte de donnees,
  et plus de garde-fou fragile base sur la comparaison des comptes totaux (qui
  sautait l'enrichissement quand YGOJSON listait plus de prints — artworks
  alternatifs — que la liste Yugipedia propre).

DETECTION OVERFRAME (robuste)
  Une entree Yugipedia est consideree « extended art » si :
    - sa note (apres « // ») contient un mot-cle de _EXTENDED_KEYWORDS, OU
    - parse_set_list l'a deja taguee extended_art=True, OU
    - une de ses raretes est dans _ALWAYS_EXTENDED (Grand Master Rare) — la GMR
      n'existe qu'en Overframe, c'est le signal le plus fiable, presente meme
      quand la note « // extended art » est absente du wikitext.
  Quand une entree est extended art, TOUTES ses raretes sont Overframe.

GARDE-FOUS
  - Idempotent : on memorise le revid Yugipedia ; un set deja a jour est saute.
  - Cible : on n'enrichit que les sets demandes (whitelist au build + hook a la
    creation d'un classeur), jamais les 3300 sets (quota Yugipedia 1 req/s).
  - Robustesse : les erreurs reseau sont capturees et renvoyees en statut.

DEPENDANCE
  module.donnees.sync_reference  (client Yugipedia : resolve_set_pages,
  fetch_set_cards, throttling 1 req/s, User-Agent obligatoire).
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

import sqlite3
import time

from module.donnees import sync_reference as _yugi

try:
    from module.logger_app import log as _log
except Exception:                                  # autonomie en test
    import logging
    _log = logging.getLogger("overframe_enrichment")


# ───────────────────────────────────────────────────────────────────────────
# Whitelist des sets a traitement Overframe connus (enrichis au build).
# Format : (prefixe_set_code, nom_anglais_yugipedia, region).
# Ajouter ici tout nouveau set Overframe identifie ; le hook a la creation
# couvre de toute facon les sets non listes.
# ───────────────────────────────────────────────────────────────────────────
OVERFRAME_SETS: list[tuple[str, str, str]] = [
    ("LOCH-JP", "Limit Over Collection: The Heroes", "OCG-JP"),
    ("LOCR-JP", "Limit Over Collection: The Rivals", "OCG-JP"),
]

# Mots-cles signalant un print extended art / Overframe dans la note Yugipedia
# (partie apres « // » d'une entree de Set list). Compares en minuscules.
_EXTENDED_KEYWORDS: tuple[str, ...] = (
    "extended art", "extended artwork", "overframe", "over frame", "over flame",
)

# Raretes qui n'existent QU'EN extended art (Overframe). Filet de securite : la
# presence d'une de ces raretes suffit a marquer l'entree Overframe, meme si la
# note « // extended art » est absente du wikitext. Comparees en minuscules.
_ALWAYS_EXTENDED: set[str] = {"grand master rare", "grandmaster rare"}


# ───────────────────────────────────────────────────────────────────────────
# Schema
# ───────────────────────────────────────────────────────────────────────────

def ensure_extended_art_column(conn: sqlite3.Connection) -> bool:
    """Ajoute set_prints.extended_art si absente. Retourne True si ajoutee.
    Idempotent : sans effet si la colonne existe deja (build initial inclus)."""
    cols = {r[1] for r in conn.execute("PRAGMA table_info(set_prints)")}
    if "extended_art" not in cols:
        conn.execute(
            "ALTER TABLE set_prints ADD COLUMN extended_art INTEGER NOT NULL DEFAULT 0")
        return True
    return False


def _ensure_sync_table(conn: sqlite3.Connection) -> None:
    conn.execute("""
        CREATE TABLE IF NOT EXISTS overframe_sync (
            set_prefix TEXT PRIMARY KEY,
            revid      TEXT,
            synced_at  TEXT
        )""")


def _get_revid(conn, prefix):
    _ensure_sync_table(conn)
    row = conn.execute("SELECT revid FROM overframe_sync WHERE set_prefix=?",
                       (prefix,)).fetchone()
    return row[0] if row else None


def _set_revid(conn, prefix, revid):
    conn.execute("""INSERT INTO overframe_sync (set_prefix, revid, synced_at)
                    VALUES (?,?,?)
                    ON CONFLICT(set_prefix) DO UPDATE SET
                      revid=excluded.revid, synced_at=excluded.synced_at""",
                 (prefix, str(revid), time.strftime("%Y-%m-%d %H:%M:%S")))


# ───────────────────────────────────────────────────────────────────────────
# Classification des prints Yugipedia
# ───────────────────────────────────────────────────────────────────────────

def _entree_est_overframe(entree: dict, raretes: list[str]) -> bool:
    """True si l'entree Yugipedia decrit des prints extended art (Overframe).

    Signaux (OR) : note tagee extended art, flag extended_art du parser, ou
    presence d'une rarete _ALWAYS_EXTENDED (Grand Master Rare)."""
    note = (entree.get("note") or "").lower()
    if entree.get("extended_art"):
        return True
    if any(k in note for k in _EXTENDED_KEYWORDS):
        return True
    if any(r.lower() in _ALWAYS_EXTENDED for r in raretes):
        return True
    return False


def _classify_prints(entrees: list[dict]) -> tuple[set, set]:
    """Repartit les (numero, rarete) Yugipedia en deux ensembles.

    Retourne (normal_set, ext_set) :
      - normal_set : prints en cadre NORMAL ;
      - ext_set    : prints en cadre EXTENDED ART (Overframe).
    Une meme (numero, rarete) peut figurer dans les DEUX (rarete existant dans
    les deux cadres, ex Prismatic Secret Rare des cartes chase)."""
    normal_set: set = set()
    ext_set: set = set()
    for e in entrees:
        raretes = [r.strip() for r in e.get("raretes", []) if r and r.strip()]
        if not raretes:
            continue
        cible = ext_set if _entree_est_overframe(e, raretes) else normal_set
        for rar in raretes:
            cible.add((e["numero"], rar))
    return normal_set, ext_set


# ───────────────────────────────────────────────────────────────────────────
# Reconciliation d'un set (additive + correction de cadre)
# ───────────────────────────────────────────────────────────────────────────

def enrich_set(conn: sqlite3.Connection, set_prefix: str, set_name: str,
               region: str, force: bool = False) -> dict:
    """Complete les prints Overframe d'un set depuis Yugipedia (approche additive).

    N'AJOUTE/ne corrige QUE les variantes extended art ; ne supprime jamais de
    print existant. Retourne un dict de statut :
      {"prefix", "status", "ajoutes", "corriges", "revid"}
    status ∈ {"ok", "deja_a_jour", "aucun_overframe", "page_introuvable",
              "absent_cardinfo", "erreur"}.
    Ne leve pas : les erreurs reseau sont capturees et renvoyees en statut."""
    ensure_extended_art_column(conn)
    try:
        pages = _yugi.resolve_set_pages(set_name, region)
        if not pages:
            return {"prefix": set_prefix, "status": "page_introuvable"}

        entrees, revid = _yugi.fetch_set_cards(pages[0])

        if not force and _get_revid(conn, set_prefix) == str(revid):
            return {"prefix": set_prefix, "status": "deja_a_jour", "revid": revid}

        normal_set, ext_set = _classify_prints(entrees)
        if not ext_set:
            # Set sans aucune carte Overframe selon Yugipedia : on memorise le
            # revid (pour ne pas re-scraper) et on ne touche a rien.
            _set_revid(conn, set_prefix, revid)
            conn.commit()
            return {"prefix": set_prefix, "status": "aucun_overframe",
                    "revid": revid, "ajoutes": 0, "corriges": 0}

        # SELECT colonnes explicites + accès POSITIONNEL : la connexion passée
        # par le hook de création n'a pas forcément row_factory=sqlite3.Row.
        # (0 set_uuid, 1 set_locale_id, 2 card_uuid, 3 card_image_uuid,
        #  4 set_code, 5 rarity, 6 edition, 7 qty, 8 print_image_url,
        #  9 extended_art, 10 id)
        existing = conn.execute(
            """SELECT set_uuid, set_locale_id, card_uuid, card_image_uuid,
                      set_code, rarity, edition, qty, print_image_url,
                      extended_art, id
               FROM set_prints WHERE set_code LIKE ?""",
            (set_prefix + "%",)).fetchall()
        if not existing:
            return {"prefix": set_prefix, "status": "absent_cardinfo"}

        set_uuid  = existing[0][0]
        locale_id = existing[0][1]

        meta_by_num: dict[str, dict] = {}    # numero -> refs carte heritables
        present_ext: set = set()             # (set_code, rarity) deja en ext=1
        normal_rows: dict[tuple, int] = {}   # (set_code, rarity) -> id (ext=0)
        url_by_num_rar: dict[tuple, str] = {}
        for row in existing:
            su, li, cu, ciu, sc, ra, ed, q, pu, ext, rid = row
            meta_by_num.setdefault(sc, {
                "card_uuid": cu,
                "card_image_uuid": ciu,
                "edition": ed if ed is not None else "unlimited",
                "qty": q if q is not None else 1,
            })
            if int(ext or 0) == 1:
                present_ext.add((sc, ra))
            else:
                normal_rows.setdefault((sc, ra), rid)
            if pu and (sc, ra) not in url_by_num_rar:
                url_by_num_rar[(sc, ra)] = pu

        new_rows: list[tuple] = []
        updates: list[int] = []
        for (num, rar) in sorted(ext_set):
            if (num, rar) in present_ext:
                continue                              # deja present en Overframe
            if (num, rar) in normal_set:
                # Rarete presente dans les DEUX cadres -> ajouter la ligne
                # Overframe en conservant la normale existante.
                m = meta_by_num.get(num)
                if m:
                    new_rows.append((
                        set_uuid, locale_id, (m["card_uuid"] or ""),
                        m["card_image_uuid"], num, rar, m["edition"], m["qty"],
                        url_by_num_rar.get((num, rar)), 1))
            else:
                # Rarete Overframe-only.
                rid = normal_rows.get((num, rar))
                if rid is not None:
                    # YGOJSON l'a importee a tort en cadre normal -> corriger
                    # le flag (pas de doublon).
                    updates.append(rid)
                else:
                    m = meta_by_num.get(num)
                    if m:
                        new_rows.append((
                            set_uuid, locale_id, (m["card_uuid"] or ""),
                            m["card_image_uuid"], num, rar, m["edition"],
                            m["qty"], url_by_num_rar.get((num, rar)), 1))

        if updates:
            conn.executemany(
                "UPDATE set_prints SET extended_art=1 WHERE id=?",
                [(rid,) for rid in updates])
        if new_rows:
            conn.executemany("""
                INSERT INTO set_prints
                    (set_uuid, set_locale_id, card_uuid, card_image_uuid,
                     set_code, rarity, edition, qty, print_image_url, extended_art)
                VALUES (?,?,?,?,?,?,?,?,?,?)
            """, new_rows)
        _set_revid(conn, set_prefix, revid)
        conn.commit()

        _log.info(f"[Overframe] {set_prefix} : +{len(new_rows)} prints, "
                  f"{len(updates)} corriges (rev {revid}).")
        return {"prefix": set_prefix, "status": "ok", "revid": revid,
                "ajoutes": len(new_rows), "corriges": len(updates)}

    except Exception as e:                          # robustesse : jamais bloquant
        _log.warning(f"[Overframe] {set_prefix} : erreur {e}")
        return {"prefix": set_prefix, "status": "erreur", "erreur": str(e)}


# ───────────────────────────────────────────────────────────────────────────
# Points d'entree
# ───────────────────────────────────────────────────────────────────────────

def enrich_whitelist(conn: sqlite3.Connection, progress=None,
                     force: bool = False) -> list[dict]:
    """Enrichit tous les sets de OVERFRAME_SETS (appel au build initial).
    progress(msg:str) optionnel.
    force=True : ignore le garde-fou d'idempotence (revid) et ré-applique la
    complétion même si le set a déjà été synchronisé — utilisé par la
    correction manuelle déclenchée depuis les Options quand cardinfo.db existe
    déjà mais n'a jamais reçu (ou a partiellement reçu) les prints Overframe."""
    ensure_extended_art_column(conn)
    res = []
    for prefix, name, region in OVERFRAME_SETS:
        if progress:
            progress(f"Complétion Overframe : {prefix}…")
        res.append(enrich_set(conn, prefix, name, region, force=force))
    return res


# Index { prefixe : (nom_anglais, region) } pour le hook a la creation.
_WL_INDEX = {p: (n, r) for (p, n, r) in OVERFRAME_SETS}


def enrich_for_classeur(conn: sqlite3.Connection, set_prefix: str,
                        set_name_en: str | None = None,
                        region: str | None = None,
                        progress=None) -> dict:
    """Hook a la creation d'un classeur : enrichit le set demande si besoin.

    set_prefix : ex 'LOCR-JP' (prefixe + region du set_code).
    set_name_en / region : facultatifs ; si absents et le set est dans la
    whitelist, ils en sont deduits. Sinon, on tente une resolution Yugipedia
    a partir du nom anglais du set (a fournir par l'appelant).

    Idempotent (revid). Renvoie le dict de statut de enrich_set, ou
    {"status":"non_overframe"} si on ne sait pas resoudre le set (cas normal
    pour l'immense majorite des sets : aucun cout, aucune requete)."""
    if not set_name_en or not region:
        wl = _WL_INDEX.get(set_prefix)
        if wl:
            set_name_en, region = wl
        else:
            # Set hors whitelist et sans nom fourni : on ne scrape pas a
            # l'aveugle. L'appelant peut rappeler avec set_name_en/region.
            return {"prefix": set_prefix, "status": "non_overframe"}

    if progress:
        progress(f"Vérification Overframe : {set_prefix}…")
    return enrich_set(conn, set_prefix, set_name_en, region)


# ───────────────────────────────────────────────────────────────────────────
# Correction manuelle (depuis les Options)
# ───────────────────────────────────────────────────────────────────────────

def corriger_overframe_cardinfo(progress=None, force: bool = True) -> dict:
    """Déclenche manuellement la complétion Overframe sur cardinfo.db existant.

    PROBLEME RESOLU
      L'enrichissement Overframe ne s'exécute normalement qu'au build de la
      base OU à la création d'un classeur (hook), et il est idempotent via le
      revid Yugipedia. Resultat : si cardinfo.db existe deja et qu'un set
      Overframe a ete marque « synchronise » (ou n'a jamais ete enrichi parce
      qu'aucun classeur de ce set n'a encore ete cree), les prints Overframe
      non declares ne sont jamais corriges. Cette fonction force la
      reconciliation pour TOUS les sets de la whitelist, quel que soit l'etat
      du revid (force=True par defaut).

    Ouvre cardinfo.db via sqlite_ctx (commit/rollback geres par le contexte ;
    chaque set est aussi commit individuellement par enrich_set, donc un echec
    reseau tardif ne perd pas les sets deja traites).

    Reseau (Yugipedia, 1 req/s) : a executer hors thread UI par l'appelant.

    Retourne un dict agrege :
      {"status", "sets", "ajoutes", "corriges", "details": [<dict enrich_set>]}
      status ∈ {"ok", "rien", "cardinfo_absente", "erreur"}.
    Ne leve pas : les erreurs sont capturees et renvoyees en statut."""
    try:
        import os
        from module.centralisation_dossier import CARDINFO_DB, sqlite_ctx
    except Exception as e:                              # import defensif
        return {"status": "erreur", "erreur": f"import: {e}",
                "sets": 0, "ajoutes": 0, "corriges": 0, "details": []}

    if not os.path.isfile(CARDINFO_DB):
        return {"status": "cardinfo_absente",
                "sets": 0, "ajoutes": 0, "corriges": 0, "details": []}

    try:
        with sqlite_ctx(CARDINFO_DB) as conn:
            details = enrich_whitelist(conn, progress=progress, force=force)
    except Exception as e:
        _log.warning(f"corriger_overframe_cardinfo : {e}")
        return {"status": "erreur", "erreur": str(e),
                "sets": 0, "ajoutes": 0, "corriges": 0, "details": []}

    ajoutes  = sum(int(d.get("ajoutes", 0) or 0)  for d in details)
    corriges = sum(int(d.get("corriges", 0) or 0) for d in details)
    sets_ok  = sum(1 for d in details if d.get("status") == "ok")
    status   = "ok" if (ajoutes or corriges) else "rien"
    _log.info(f"[Overframe] correction manuelle : {sets_ok} set(s) modifie(s), "
              f"+{ajoutes} prints, {corriges} corriges.")
    return {"status": status, "sets": sets_ok,
            "ajoutes": ajoutes, "corriges": corriges, "details": details}
