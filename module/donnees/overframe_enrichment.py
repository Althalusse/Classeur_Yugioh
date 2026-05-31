# -*- coding: utf-8 -*-
"""
overframe_enrichment.py — Complétion des prints "extended art" (Overframe) via Yugipedia.

PROBLEME RESOLU
  La source YGOJSON (cardinfo.db) est INCOMPLETE pour les sets a traitement
  Overframe : pour LOCR-JP001 elle ne liste que 3 prints (Ultra / Secret /
  Prismatic Secret) au lieu de 5. Manquent : l'Ultra Rare *Overframe* et la
  Grand Master Rare. YGOJSON ne distingue pas non plus le cadre (une meme
  rarete existe en cadre normal ET en Overframe). Comme l'application place les
  cartes de maniere sequentielle, tout print manquant DECALE la grille.

STRATEGIE (validee en reel)
  Pour un set donne, Yugipedia est la LISTE AUTORITAIRE des prints :
  chaque (numero, rarete, extended_art) y est present. On reconstruit les
  set_prints du set a partir de Yugipedia, en HERITANT les metadonnees
  (card_uuid, card_image_uuid, set_uuid, set_locale_id, edition, qty,
  print_image_url) depuis les lignes cardinfo existantes du meme set
  (tous les prints d'un meme numero partagent le meme artwork).

GARDE-FOUS
  - Si Yugipedia renvoie MOINS de prints que cardinfo -> on N'ECRASE PAS
    (anomalie loggee). On ne perd jamais de donnees.
  - Idempotent : on memorise le revid Yugipedia ; un set deja a jour est saute.
  - Cible : on n'enrichit que les sets demandes (whitelist au build + hook a la
    creation d'un classeur), jamais les 3300 sets (quota Yugipedia 1 req/s).

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

# Raretes connues comme "extended art" meme si la ligne Yugipedia n'est pas
# taguee (filet de securite ; la detection principale reste le tag wikitext).
_ALWAYS_EXTENDED = {"grand master rare"}


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
# Reconciliation d'un set
# ───────────────────────────────────────────────────────────────────────────

def _authoritative_prints(entrees: list[dict]) -> list[tuple]:
    """Transforme les entrees Yugipedia en liste (numero, rarete, extended_art)."""
    out = []
    for e in entrees:
        for rar in e["raretes"]:
            ext = 1 if (e["extended_art"] or rar.strip().lower() in _ALWAYS_EXTENDED) else 0
            out.append((e["numero"], rar.strip(), ext))
    return out


def enrich_set(conn: sqlite3.Connection, set_prefix: str, set_name: str,
               region: str, force: bool = False) -> dict:
    """Reconstruit les set_prints d'un set depuis Yugipedia (liste autoritaire).

    Retourne un dict de statut :
      {"prefix", "status", "avant", "apres", "revid"}
    status ∈ {"ok", "deja_a_jour", "page_introuvable", "absent_cardinfo",
              "anomalie_moins_de_prints", "erreur"}.
    Ne leve pas : les erreurs reseau sont capturees et renvoyees en statut."""
    ensure_extended_art_column(conn)
    try:
        pages = _yugi.resolve_set_pages(set_name, region)
        if not pages:
            return {"prefix": set_prefix, "status": "page_introuvable"}

        entrees, revid = _yugi.fetch_set_cards(pages[0])

        if not force and _get_revid(conn, set_prefix) == str(revid):
            return {"prefix": set_prefix, "status": "deja_a_jour", "revid": revid}

        autoritaire = _authoritative_prints(entrees)
        if not autoritaire:
            return {"prefix": set_prefix, "status": "page_introuvable"}

        # SELECT colonnes explicites + accès POSITIONNEL : la connexion passée
        # par le hook de création n'a pas forcément row_factory=sqlite3.Row.
        # (0 set_uuid, 1 set_locale_id, 2 card_uuid, 3 card_image_uuid,
        #  4 set_code, 5 rarity, 6 edition, 7 qty, 8 print_image_url)
        existing = conn.execute(
            """SELECT set_uuid, set_locale_id, card_uuid, card_image_uuid,
                      set_code, rarity, edition, qty, print_image_url
               FROM set_prints WHERE set_code LIKE ?""",
            (set_prefix + "%",)).fetchall()
        if not existing:
            return {"prefix": set_prefix, "status": "absent_cardinfo"}

        # GARDE-FOU : ne jamais reduire le set.
        if len(autoritaire) < len(existing):
            _log.warning(f"[Overframe] {set_prefix} : Yugipedia {len(autoritaire)} < "
                         f"cardinfo {len(existing)} -> enrichissement ignore (anomalie).")
            return {"prefix": set_prefix, "status": "anomalie_moins_de_prints",
                    "avant": len(existing), "apres": len(existing)}

        set_uuid  = existing[0][0]
        locale_id = existing[0][1]

        meta_by_num: dict[str, dict] = {}
        url_by_num_rar: dict[tuple, str] = {}
        for row in existing:
            su, li, cu, ciu, sc, ra, ed, q, pu = row
            meta_by_num.setdefault(sc, {
                "card_uuid": cu,
                "card_image_uuid": ciu,
                "edition": ed if ed is not None else "unlimited",
                "qty": q if q is not None else 1,
            })
            url_by_num_rar[(sc, ra)] = pu

        # Reconstruction transactionnelle.
        conn.execute("DELETE FROM set_prints WHERE set_code LIKE ?",
                     (set_prefix + "%",))
        rows = []
        for num, rar, ext in autoritaire:
            m = meta_by_num.get(num, {
                "card_uuid": "", "card_image_uuid": None,
                "edition": "unlimited", "qty": 1})
            rows.append((
                set_uuid, locale_id, (m["card_uuid"] or ""), m["card_image_uuid"],
                num, rar, m["edition"], m["qty"],
                url_by_num_rar.get((num, rar)), ext,
            ))
        conn.executemany("""
            INSERT INTO set_prints
                (set_uuid, set_locale_id, card_uuid, card_image_uuid,
                 set_code, rarity, edition, qty, print_image_url, extended_art)
            VALUES (?,?,?,?,?,?,?,?,?,?)
        """, rows)
        _set_revid(conn, set_prefix, revid)
        conn.commit()

        _log.info(f"[Overframe] {set_prefix} : {len(existing)} -> {len(rows)} prints "
                  f"(rev {revid}).")
        return {"prefix": set_prefix, "status": "ok", "revid": revid,
                "avant": len(existing), "apres": len(rows)}

    except Exception as e:                          # robustesse : jamais bloquant
        _log.warning(f"[Overframe] {set_prefix} : erreur {e}")
        return {"prefix": set_prefix, "status": "erreur", "erreur": str(e)}


# ───────────────────────────────────────────────────────────────────────────
# Points d'entree
# ───────────────────────────────────────────────────────────────────────────

def enrich_whitelist(conn: sqlite3.Connection, progress=None) -> list[dict]:
    """Enrichit tous les sets de OVERFRAME_SETS (appel au build initial).
    progress(msg:str) optionnel."""
    ensure_extended_art_column(conn)
    res = []
    for prefix, name, region in OVERFRAME_SETS:
        if progress:
            progress(f"Complétion Overframe : {prefix}…")
        res.append(enrich_set(conn, prefix, name, region))
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
