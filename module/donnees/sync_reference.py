# -*- coding: utf-8 -*-
"""
Synchronisation de la base de REFERENCE Yu-Gi-Oh (cartes + sets + prints).

Strategie (validee en reel) :
  - Cartes / metadonnees / images : YGOPRODeck  -> 1 seule requete pour TOUT
    (cardinfo.php sans filtre), rafraichie uniquement si checkDBVer.php change.
  - Sets / numeros / raretes / variantes (Overframe...) : Yugipedia (1 req/s)
    -> seul capable de donner la granularite complete par print.
  - Reconciliation : passcode 8 chiffres ; a defaut, liaison par nom (best effort).

A LIRE :
  * Cette synchro fait du reseau bloquant (1 req/s cote Yugipedia) -> LANCER
    DANS UN THREAD, jamais sur le thread UI. Utiliser run_sync_in_thread().
  * Renseigner CONTACT ci-dessous (User-Agent descriptif OBLIGATOIRE cote wiki,
    sinon blocage possible).
  * Les images ne sont PAS telechargees ici (hotlink interdit) : on stocke l'URL,
    le telechargement reste a la charge du worker d'images existant du projet.
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

import html
import json
import re
import sqlite3
import threading
import time

import requests

# ------------------------------------------------------------------ #
# Configuration                                                       #
# ------------------------------------------------------------------ #

CONTACT = "yugioh-binder; https://github.com/yugioh-binder"  # surchargeable ; identifie l'app
APP_UA = "YuGiOhBinder/1.0"

YGOPRO_VER = "https://db.ygoprodeck.com/api/v7/checkDBVer.php"
YGOPRO_CARDS = "https://db.ygoprodeck.com/api/v7/cardinfo.php"
YUGI_API = "https://yugipedia.com/api.php"

HTTP_TIMEOUT = 30
BULK_TIMEOUT = 180                            # le bulk cartes pese plusieurs dizaines de Mo
MAX_RETRIES = 3
YUGI_MIN_INTERVAL = 1.1                       # quota Yugipedia : 1 req/s -> marge de securite
INFO_BATCH = 50                               # prop=info : 50 titres max (non-bot)


def _user_agent() -> dict:
    return {"User-Agent": f"{APP_UA} ({CONTACT})"}


def _check_contact():
    # Garde minimale : un User-Agent descriptif non vide est requis par
    # Yugipedia. Le défaut (APP_UA + CONTACT) le satisfait toujours ; on ne
    # bloque donc jamais l'utilisateur. Surcharger CONTACT/APP_UA reste possible.
    if not (APP_UA and APP_UA.strip()):
        raise RuntimeError("APP_UA vide : User-Agent descriptif requis pour Yugipedia.")


# ------------------------------------------------------------------ #
# Couche HTTP (retry/backoff + limiteur de debit Yugipedia)           #
# ------------------------------------------------------------------ #

_yugi_lock = threading.Lock()
_yugi_last = [0.0]


def _throttle_yugi():
    """Garantit >= 1 req/s vers Yugipedia, meme avec plusieurs threads."""
    with _yugi_lock:
        dt = time.monotonic() - _yugi_last[0]
        if dt < YUGI_MIN_INTERVAL:
            time.sleep(YUGI_MIN_INTERVAL - dt)
        _yugi_last[0] = time.monotonic()


def _request(url, params=None, headers=None, timeout=HTTP_TIMEOUT, throttle=False):
    """GET avec retry/backoff sur erreurs transitoires (429 / 5xx)."""
    last_exc = None
    for essai in range(1, MAX_RETRIES + 1):
        if throttle:
            _throttle_yugi()
        try:
            r = requests.get(url, params=params, headers=headers, timeout=timeout)
            if r.status_code == 429:
                attente = int(r.headers.get("Retry-After", essai * 5))
                time.sleep(attente)
                continue
            if 500 <= r.status_code < 600:
                time.sleep(essai * 2)
                continue
            r.raise_for_status()
            return r
        except requests.RequestException as e:
            last_exc = e
            time.sleep(essai * 2)
    raise RuntimeError(f"Echec requete apres {MAX_RETRIES} essais : {url} ({last_exc})")


def _yugi(params):
    """Appel API Yugipedia (JSON, formatversion 2, debit limite)."""
    _check_contact()
    p = {**params, "format": "json", "formatversion": "2"}
    r = _request(YUGI_API, params=p, headers=_user_agent(), throttle=True)
    data = r.json()
    if "error" in data:
        raise ValueError(f"Erreur API Yugipedia : {data['error'].get('info', data['error'])}")
    return data


# ------------------------------------------------------------------ #
# Base de donnees                                                     #
# ------------------------------------------------------------------ #

SCHEMA = """
CREATE TABLE IF NOT EXISTS cards (
    passcode    INTEGER PRIMARY KEY,        -- id YGOPRODeck = mot de passe 8 chiffres
    nom         TEXT NOT NULL,
    type        TEXT,
    frame_type  TEXT,
    atk         INTEGER,
    def         INTEGER,
    niveau      INTEGER,
    attribut    TEXT,
    race        TEXT,
    archetype   TEXT,
    description TEXT,
    image_url   TEXT,                        -- source (a telecharger 1x par le worker)
    image_path  TEXT,                        -- chemin LOCAL (rempli par le worker)
    ocg_date    TEXT,
    tcg_date    TEXT,
    formats     TEXT,                        -- ex 'TCG,OCG,Master Duel'
    source      TEXT DEFAULT 'ygoprodeck',
    maj_le      TEXT
);

CREATE TABLE IF NOT EXISTS sets (
    set_id      TEXT PRIMARY KEY,            -- prefixe + region, ex 'LOCH-JP'
    nom         TEXT NOT NULL,
    region      TEXT,                        -- 'OCG-JP', 'OCG-KR', 'EN'...
    page_titre  TEXT,                        -- titre EXACT Yugipedia (resync)
    source      TEXT DEFAULT 'yugipedia',
    maj_le      TEXT
);

-- Capture chaque print physique (c'est ici que vivent les variantes Overframe).
CREATE TABLE IF NOT EXISTS prints (
    numero        TEXT NOT NULL,             -- ex 'LOCH-JP001'
    set_id        TEXT NOT NULL,
    passcode      INTEGER,                   -- NULL si carte absente de YGOPRODeck
    nom           TEXT NOT NULL,             -- nom imprime (fallback si passcode NULL)
    rarete        TEXT NOT NULL DEFAULT '',
    extended_art  INTEGER NOT NULL DEFAULT 0,-- 1 = Overframe / extended art
    note          TEXT DEFAULT '',
    PRIMARY KEY (numero, rarete, extended_art)
);

CREATE INDEX IF NOT EXISTS idx_prints_set      ON prints(set_id);
CREATE INDEX IF NOT EXISTS idx_prints_passcode ON prints(passcode);
CREATE INDEX IF NOT EXISTS idx_cards_nom       ON cards(nom);

CREATE TABLE IF NOT EXISTS sync_state (
    ressource   TEXT PRIMARY KEY,            -- 'ygoprodeck:cards' | 'yugipedia:<titre>'
    version     TEXT,                        -- database_version | lastrevid
    synchro_le  TEXT
);
"""


def open_db(chemin: str) -> sqlite3.Connection:
    """Connexion durcie (anti 'database is locked' + perfs)."""
    conn = sqlite3.connect(chemin, timeout=10)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA busy_timeout=5000")
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA synchronous=NORMAL")
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def init_schema(conn: sqlite3.Connection):
    conn.executescript(SCHEMA)
    conn.commit()


def _now() -> str:
    return time.strftime("%Y-%m-%d %H:%M:%S")


def _get_state(conn, ressource):
    row = conn.execute("SELECT version FROM sync_state WHERE ressource=?",
                       (ressource,)).fetchone()
    return row["version"] if row else None


def _set_state(conn, ressource, version):
    conn.execute(
        """INSERT INTO sync_state (ressource, version, synchro_le) VALUES (?,?,?)
           ON CONFLICT(ressource) DO UPDATE SET version=excluded.version,
                                                synchro_le=excluded.synchro_le""",
        (ressource, str(version), _now()))


# ------------------------------------------------------------------ #
# YGOPRODeck : cartes (metadonnees + images)                          #
# ------------------------------------------------------------------ #

def ygopro_db_version() -> str:
    return _request(YGOPRO_VER, timeout=HTTP_TIMEOUT).json()[0]["database_version"]


def sync_cards(conn: sqlite3.Connection, force: bool = False, progress=None) -> int:
    """Met a jour la table cards si la base YGOPRODeck a change.
    Retourne le nombre de cartes ecrites (0 si deja a jour)."""
    def log(m):
        if progress:
            progress(m)

    version = ygopro_db_version()
    if not force and _get_state(conn, "ygoprodeck:cards") == version:
        log("Cartes : deja a jour.")
        return 0

    log("Cartes : telechargement complet YGOPRODeck...")
    data = _request(YGOPRO_CARDS, params={"misc": "yes"}, timeout=BULK_TIMEOUT).json()["data"]
    log(f"Cartes : {len(data)} recues, ecriture en base...")

    lignes = []
    for c in data:
        mi = (c.get("misc_info") or [{}])[0]
        imgs = c.get("card_images") or [{}]
        lignes.append({
            "passcode": c["id"], "nom": c["name"], "type": c.get("type"),
            "frame_type": c.get("frameType"), "atk": c.get("atk"), "def": c.get("def"),
            "niveau": c.get("level"), "attribut": c.get("attribute"), "race": c.get("race"),
            "archetype": c.get("archetype"), "description": c.get("desc"),
            "image_url": imgs[0].get("image_url"),
            "ocg_date": mi.get("ocg_date"), "tcg_date": mi.get("tcg_date"),
            "formats": ",".join(mi.get("formats", []) or []),
            "maj": _now(),
        })

    conn.executemany(
        """INSERT INTO cards (passcode, nom, type, frame_type, atk, def, niveau,
                              attribut, race, archetype, description, image_url,
                              ocg_date, tcg_date, formats, maj_le)
           VALUES (:passcode,:nom,:type,:frame_type,:atk,:def,:niveau,:attribut,:race,
                   :archetype,:description,:image_url,:ocg_date,:tcg_date,:formats,:maj)
           ON CONFLICT(passcode) DO UPDATE SET
             nom=excluded.nom, type=excluded.type, frame_type=excluded.frame_type,
             atk=excluded.atk, def=excluded.def, niveau=excluded.niveau,
             attribut=excluded.attribut, race=excluded.race, archetype=excluded.archetype,
             description=excluded.description, image_url=excluded.image_url,
             ocg_date=excluded.ocg_date, tcg_date=excluded.tcg_date,
             formats=excluded.formats, maj_le=excluded.maj_le""",
        lignes)
    _set_state(conn, "ygoprodeck:cards", version)
    conn.commit()
    log(f"Cartes : {len(lignes)} a jour (DB v{version}).")
    return len(lignes)


# ------------------------------------------------------------------ #
# Yugipedia : sets / prints                                           #
# ------------------------------------------------------------------ #

def resolve_set_pages(set_name: str, region: str | None = None) -> list[str]:
    """Resout le(s) titre(s) de page 'Set Card Lists:' via prefixsearch
    (la recherche plein-texte ne couvre PAS ce namespace de maniere fiable).
    region : 'OCG-JP', 'OCG-KR', 'EN'... pour filtrer le suffixe."""
    data = _yugi({"action": "query", "list": "prefixsearch",
                  "pssearch": f"Set Card Lists:{set_name}", "pslimit": 20})
    titres = [h["title"] for h in data["query"].get("prefixsearch", [])]
    if region:
        titres = [t for t in titres if f"({region})" in t]
    return titres


def _extract_set_list_blocks(wikitext: str) -> list[str]:
    """Corps de chaque {{Set list ...}} (compteur d'accolades -> gere l'imbrication)."""
    needle = "{{Set list"
    low = wikitext.lower()
    nl = needle.lower()
    blocs, idx = [], 0
    while (s := low.find(nl, idx)) != -1:
        depth, j = 0, s
        while j < len(wikitext) - 1:
            pair = wikitext[j:j + 2]
            if pair == "{{":
                depth += 1; j += 2
            elif pair == "}}":
                depth -= 1; j += 2
                if depth == 0:
                    break
            else:
                j += 1
        blocs.append(wikitext[s + len(needle):j - 2])
        idx = j
    return blocs


def _clean(s: str) -> str:
    s = re.sub(r"\[\[(?:[^\]|]*\|)?([^\]]+)\]\]", r"\1", s)   # [[A|B]] -> B ; [[A]] -> A
    return html.unescape(s).strip()


def parse_set_list(wikitext: str) -> list[dict]:
    """Parse les entrees de tous les blocs {{Set list}}.
    Entrees separees par RETOUR LIGNE ; champs separes par ';' ;
    options apres '//'. Herite des parametres par defaut de l'en-tete."""
    cartes = []
    for corps in _extract_set_list_blocks(wikitext):
        defauts = {}
        for ligne in corps.split("\n"):
            ligne = ligne.strip()
            if not ligne:
                continue
            if ";" not in ligne:                              # ligne de parametres
                for tok in ligne.split("|"):
                    if "=" in tok:
                        k, v = tok.split("=", 1)
                        defauts[k.strip()] = v.strip()
                continue
            entree = ligne.lstrip("|").strip()                # ligne de carte
            main, _, opts = entree.partition("//")
            champs = [c.strip() for c in main.split(";")]
            numero = champs[0] if champs and champs[0] else ""
            nom = _clean(champs[1]) if len(champs) > 1 else ""
            raretes = champs[2] if len(champs) > 2 and champs[2] else defauts.get("rarities", "")
            note = opts.strip()
            cartes.append({
                "numero": numero,
                "nom": nom,
                "raretes": [r.strip() for r in raretes.split(",") if r.strip()],
                "extended_art": "extended art" in note.lower(),
                "note": note,
            })
    return cartes


def _set_id_from_numero(numero: str) -> str:
    """'LOCH-JP001' -> 'LOCH-JP' (prefixe + region)."""
    m = re.match(r"^(.*?-[A-Z]+)\d", numero)
    return m.group(1) if m else numero.rsplit("-", 1)[0] if "-" in numero else numero


def _name_to_set(titre: str) -> tuple[str, str]:
    """'Set Card Lists:Foo (OCG-JP)' -> ('Foo', 'OCG-JP')."""
    base = titre.split(":", 1)[1] if ":" in titre else titre
    m = re.search(r"\(([^)]+)\)\s*$", base)
    region = m.group(1) if m else ""
    nom = re.sub(r"\s*\([^)]+\)\s*$", "", base).strip()
    return nom, region


def fetch_set_cards(titre: str) -> tuple[list[dict], int]:
    """Recupere wikitext + revid via action=parse, puis parse. Suit les redirections."""
    data = _yugi({"action": "parse", "page": titre, "prop": "wikitext|revid", "redirects": "true"})
    parse = data["parse"]
    return parse_set_list(parse["wikitext"]), parse.get("revid")


def import_set(conn: sqlite3.Connection, titre: str, progress=None) -> dict:
    """Importe un set complet (sets + prints), relie les passcodes par nom,
    enregistre le revid. Commit a la fin (reprise possible)."""
    def log(m):
        if progress:
            progress(m)

    entrees, revid = fetch_set_cards(titre)
    if not entrees:
        log(f"  [VIDE] {titre} -> aucun bloc {{Set list}} exploitable (a verifier)")
        return {"titre": titre, "prints": 0, "vide": True}

    nom_set, region = _name_to_set(titre)
    set_id = _set_id_from_numero(entrees[0]["numero"]) or nom_set

    conn.execute(
        """INSERT INTO sets (set_id, nom, region, page_titre, maj_le) VALUES (?,?,?,?,?)
           ON CONFLICT(set_id) DO UPDATE SET nom=excluded.nom, region=excluded.region,
                                             page_titre=excluded.page_titre,
                                             maj_le=excluded.maj_le""",
        (set_id, nom_set, region, titre, _now()))

    n = 0
    for e in entrees:
        raretes = e["raretes"] or [""]                        # ne jamais perdre une carte
        ext = 1 if e["extended_art"] else 0
        for rar in raretes:
            conn.execute(
                """INSERT INTO prints (numero, set_id, nom, rarete, extended_art, note)
                   VALUES (?,?,?,?,?,?)
                   ON CONFLICT(numero, rarete, extended_art) DO UPDATE SET
                     set_id=excluded.set_id, nom=excluded.nom, note=excluded.note""",
                (e["numero"], set_id, e["nom"], rar, ext, e["note"]))
            n += 1

    # nettoyage : supprime un print de rarete vide SI une rarete renseignee
    # existe pour le meme (numero, extended_art). Une carte qui n'a QUE la
    # ligne vide est conservee (on ne perd jamais une carte).
    conn.execute(
        """DELETE FROM prints
           WHERE set_id=:sid AND rarete=''
             AND EXISTS (SELECT 1 FROM prints p2
                         WHERE p2.numero = prints.numero
                           AND p2.extended_art = prints.extended_art
                           AND p2.rarete <> '')""", {"sid": set_id})

    # liaison passcode par nom (best effort ; NULL si carte absente de YGOPRODeck)
    conn.execute(
        """UPDATE prints SET passcode = (
               SELECT c.passcode FROM cards c
               WHERE lower(c.nom) = lower(prints.nom) LIMIT 1)
           WHERE set_id=? AND passcode IS NULL""", (set_id,))

    _set_state(conn, f"yugipedia:{titre}", revid)
    conn.commit()
    log(f"  [OK] {set_id} : {len(entrees)} entrees -> {n} prints (rev {revid})")
    return {"titre": titre, "set_id": set_id, "prints": n, "vide": False}


def sets_to_resync(conn: sqlite3.Connection, titres: list[str]) -> list[str]:
    """Retourne les titres dont la revision a change depuis le dernier import
    (1 requete pour 50 titres)."""
    a_faire = []
    for i in range(0, len(titres), INFO_BATCH):
        lot = titres[i:i + INFO_BATCH]
        data = _yugi({"action": "query", "prop": "info",
                      "titles": "|".join(lot), "redirects": "true"})
        # mapping titre demande -> titre canonique (redirections/normalisation)
        for p in data["query"]["pages"]:
            if p.get("missing"):
                continue
            rev = str(p.get("lastrevid"))
            connu = _get_state(conn, f"yugipedia:{p['title']}")
            if connu != rev:
                a_faire.append(p["title"])
    return a_faire


def sync_sets(conn: sqlite3.Connection, titres: list[str], progress=None) -> dict:
    """Importe/maj uniquement les sets modifies. Un set en echec n'arrete pas le reste."""
    def log(m):
        if progress:
            progress(m)

    a_faire = sets_to_resync(conn, titres)
    log(f"Sets : {len(a_faire)}/{len(titres)} a (re)synchroniser.")
    ok, vides, echecs = 0, [], []
    for t in a_faire:
        try:
            res = import_set(conn, t, progress=progress)
            if res["vide"]:
                vides.append(t)
            else:
                ok += 1
        except Exception as e:                                # robustesse : on continue
            echecs.append((t, str(e)))
            log(f"  [ECHEC] {t} : {e}")
    return {"importes": ok, "vides": vides, "echecs": echecs}


# ------------------------------------------------------------------ #
# Orchestration                                                       #
# ------------------------------------------------------------------ #

def full_sync(chemin_db: str, set_titres: list[str] | None = None,
              progress=None) -> dict:
    """Synchro complete : cartes (si besoin) + sets fournis. BLOQUANT -> thread."""
    conn = open_db(chemin_db)
    try:
        init_schema(conn)
        n_cards = sync_cards(conn, progress=progress)
        res_sets = sync_sets(conn, set_titres or [], progress=progress) if set_titres else {}
        return {"cartes": n_cards, "sets": res_sets}
    finally:
        conn.close()


def run_sync_in_thread(chemin_db: str, set_titres: list[str] | None = None,
                       progress=None, on_done=None) -> threading.Thread:
    """Lance full_sync dans un thread daemon (ne bloque pas l'UI).
    progress(msg:str) et on_done(resultat|exception) sont appeles depuis le thread :
    cote Tkinter, re-router via widget.after(0, ...)."""
    def _run():
        try:
            res = full_sync(chemin_db, set_titres, progress=progress)
            if on_done:
                on_done(res)
        except Exception as e:
            if on_done:
                on_done(e)

    th = threading.Thread(target=_run, name="sync_reference", daemon=True)
    th.start()
    return th


# ------------------------------------------------------------------ #
# Test rapide en CLI (optionnel) : python sync_reference.py           #
# ------------------------------------------------------------------ #

if __name__ == "__main__":
    import sys

    db_path = sys.argv[1] if len(sys.argv) > 1 else "cardinfo.db"
    nom = sys.argv[2] if len(sys.argv) > 2 else "Limit Over Collection: The Heroes"
    region = sys.argv[3] if len(sys.argv) > 3 else "OCG-JP"

    print(f"DB={db_path}  set='{nom}'  region={region}")
    pages = resolve_set_pages(nom, region)
    print("Pages resolues :", pages)
    if pages:
        print(full_sync(db_path, pages, progress=lambda m: print(" .", m)))
