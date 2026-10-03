// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! La passe Overframe, rejouée sur les tirages réels.
//!
//! `outils/oracle_overframe.py` extrait des deux installations l'état de
//! `set_prints` **avant** et **après** la passe :
//!
//! | | V1.0.3 | V1.0.4 |
//! |---|---|---|
//! | `LOCR-JP` | 282 tirages, aucun Overframe | 318 tirages, dont 54 Overframe |
//! | `LOCH-JP` | 279 tirages | 279 tirages, aucun Overframe |
//!
//! Aucune clé de la V1.0.3 n'a disparu de la V1.0.4, et 282 + 36 = 318 : les
//! 282 lignes **sont** l'état d'entrée exact de la passe qu'a subie la V1.0.4.
//! L'épreuve consiste donc à les rejouer et à exiger les 318 autres, colonne
//! par colonne, identifiant par identifiant.
//!
//! # La chaîne complète, sans raccourci
//!
//! Le classement n'est plus reconstruit depuis le résultat : il est **lu du
//! wikitext réel** de Yugipedia, à la révision qu'a vue la V1.0.4
//! (`5945579`), par le parser Rust puis par `classer`. La chaîne éprouvée va
//! donc du texte du wiki jusqu'aux lignes de la base :
//!
//! ```text
//! wikitext (9 931 octets)  →  parse_set_list  →  classer  →  reconcilier
//!        rev 5945579              98 entrées     264 + 54      282 → 318
//! ```
//!
//! Les trois branches de la réconciliation sont exercées par les données
//! réelles :
//!
//! | rareté | cas | effet attendu |
//! |---|---|---|
//! | Grand Master Rare | Overframe seule, absente de la base | 18 ajouts, **sans URL** — aucune ligne de cette rareté d'où hériter |
//! | Ultra Rare | présente dans les deux cadres | 18 ajouts, URL héritée, la normale conservée |
//! | Prismatic Secret Rare | Overframe seule, importée à tort en cadre normal | 18 drapeaux corrigés, aucune ligne créée |

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use rusqlite::Connection;
use serde::Deserialize;

use ygo_app::overframe::{classer, reconcilier, Bilan, Classement};
use ygo_sources::yugipedia::parse_set_list;

#[derive(Deserialize)]
struct Fixture {
    prefix: String,
    avant: Etat,
    apres: Etat,
}

#[derive(Deserialize)]
struct Etat {
    prints: Vec<Tirage>,
}

/// Une ligne de `set_prints`, telle que la base réelle la porte.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
struct Tirage {
    id: i64,
    set_uuid: String,
    set_locale_id: i64,
    card_uuid: String,
    card_image_uuid: Option<String>,
    set_code: String,
    rarity: String,
    edition: Option<String>,
    qty: Option<i64>,
    print_image_url: Option<String>,
    extended_art: i64,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn lire_json<T: serde::de::DeserializeOwned>(chemin: PathBuf) -> T {
    let texte = std::fs::read_to_string(&chemin)
        .unwrap_or_else(|e| panic!("lecture de {} : {e}", chemin.display()));
    serde_json::from_str(&texte)
        .unwrap_or_else(|e| panic!("fixture {} illisible : {e}", chemin.display()))
}

fn fixture(nom: &str) -> Fixture {
    lire_json(fixtures().join("oracle/overframe").join(nom))
}

#[derive(Deserialize)]
struct Capture {
    revid: String,
    wikitext: String,
}

/// Le classement **réel**, lu du wikitext Yugipedia par la chaîne Rust.
///
/// `revid_attendu` n'est pas décoratif : il ancre la fixture à la révision
/// qu'a vue la construction de base dont on rejoue le résultat. Une recapture
/// sur une page modifiée ferait tomber cette assertion, au lieu de comparer en
/// silence à autre chose.
fn classement_reel(fichier: &str, revid_attendu: &str) -> Classement {
    let capture: Capture = lire_json(fixtures().join("net/yugipedia").join(fichier));
    assert_eq!(
        capture.revid, revid_attendu,
        "la capture doit porter la révision qu'a vue la construction de base"
    );
    classer(&parse_set_list(&capture.wikitext))
}

/// Reconstruit la base dans l'état d'avant la passe.
///
/// Les identifiants sont **imposés** : ce sont ceux de la base réelle, et la
/// suite d'auto-incrément est repositionnée sur la valeur qu'avait la vraie
/// table. Sans ça, les lignes ajoutées ne recevraient pas les mêmes
/// identifiants — la table complète compte 326 414 lignes quand `LOCR-JP`
/// s'arrête à 323 170, et c'est bien à 326 415 que la V1.0.4 a repris.
fn base_avant(f: &Fixture) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE set_prints (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            set_uuid        TEXT NOT NULL,
            set_locale_id   INTEGER NOT NULL,
            card_uuid       TEXT NOT NULL,
            card_image_uuid TEXT,
            set_code        TEXT,
            rarity          TEXT,
            edition         TEXT,
            qty             INTEGER DEFAULT 1,
            print_image_url TEXT,
            extended_art    INTEGER NOT NULL DEFAULT 0)",
    )
    .unwrap();

    for t in &f.avant.prints {
        conn.execute(
            "INSERT INTO set_prints
                (id, set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code,
                 rarity, edition, qty, print_image_url, extended_art)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                t.id,
                &t.set_uuid,
                t.set_locale_id,
                &t.card_uuid,
                &t.card_image_uuid,
                &t.set_code,
                &t.rarity,
                &t.edition,
                t.qty,
                &t.print_image_url,
                t.extended_art
            ],
        )
        .unwrap();
    }

    // Repositionne l'auto-incrément sur la borne réelle : le plus petit
    // identifiant créé par la passe, moins un.
    let connus: BTreeSet<i64> = f.avant.prints.iter().map(|t| t.id).collect();
    if let Some(premier_nouveau) = f
        .apres
        .prints
        .iter()
        .map(|t| t.id)
        .filter(|id| !connus.contains(id))
        .min()
    {
        conn.execute(
            "UPDATE sqlite_sequence SET seq = ?1 WHERE name = 'set_prints'",
            [premier_nouveau - 1],
        )
        .unwrap();
    }
    conn
}

/// Le classement tel que la base d'arrivée le raconte — sert de contrôle
/// croisé du classement lu sur le wiki, pas d'entrée aux épreuves.
fn classement_depuis(f: &Fixture) -> Classement {
    let mut c = Classement::default();
    for t in &f.apres.prints {
        let cle = (t.set_code.clone(), t.rarity.clone());
        if t.extended_art == 1 {
            c.extended.insert(cle);
        } else {
            c.normal.insert(cle);
        }
    }
    c
}

fn lire(conn: &Connection) -> Vec<Tirage> {
    let mut stmt = conn
        .prepare(
            "SELECT id, set_uuid, set_locale_id, card_uuid, card_image_uuid, set_code,
                    rarity, edition, qty, print_image_url, extended_art
             FROM set_prints ORDER BY id",
        )
        .unwrap();
    stmt.query_map([], |l| {
        Ok(Tirage {
            id: l.get(0)?,
            set_uuid: l.get(1)?,
            set_locale_id: l.get(2)?,
            card_uuid: l.get(3)?,
            card_image_uuid: l.get(4)?,
            set_code: l.get(5)?,
            rarity: l.get(6)?,
            edition: l.get(7)?,
            qty: l.get(8)?,
            print_image_url: l.get(9)?,
            extended_art: l.get(10)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn comparer(obtenu: &[Tirage], attendu: &[Tirage], etiquette: &str) {
    assert_eq!(
        obtenu.len(),
        attendu.len(),
        "{etiquette} : {} tirages produits contre {} attendus",
        obtenu.len(),
        attendu.len()
    );
    for (o, a) in obtenu.iter().zip(attendu) {
        assert_eq!(
            o, a,
            "{etiquette} : divergence sur le tirage {} ({} / {})",
            a.id, a.set_code, a.rarity
        );
    }
}

#[test]
fn locr_jp_passe_de_282_a_318_tirages() {
    let f = fixture("bases_LOCR-JP.json");
    assert_eq!(f.prefix, "LOCR-JP");
    assert_eq!(f.avant.prints.len(), 282);
    assert_eq!(f.apres.prints.len(), 318);
    assert!(
        f.avant.prints.iter().all(|t| t.extended_art == 0),
        "l'état d'entrée ne doit porter aucun Overframe"
    );

    let classement = classement_reel("wikitext_LOCR-JP_rev5945579.json", "5945579");
    assert_eq!(classement.normal.len(), 264);
    assert_eq!(classement.extended.len(), 54);

    let conn = base_avant(&f);
    let bilan = reconcilier(&conn, &f.prefix, &classement)
        .unwrap()
        .expect("le set est présent en base");

    assert_eq!(
        bilan,
        Bilan {
            ajoutes: 36,
            corriges: 18
        },
        "le journal de la V1.0.4 dit « +36 prints, 18 corriges »"
    );
    comparer(&lire(&conn), &f.apres.prints, "LOCR-JP");
}

#[test]
fn les_trois_branches_sont_exercees_par_les_donnees_reelles() {
    let f = fixture("bases_LOCR-JP.json");
    let conn = base_avant(&f);
    let classement = classement_reel("wikitext_LOCR-JP_rev5945579.json", "5945579");
    reconcilier(&conn, &f.prefix, &classement).unwrap();

    let compter = |sql: &str| -> i64 { conn.query_row(sql, [], |l| l.get(0)).unwrap() };
    let compter_avant = |rarete: &str| -> i64 {
        f.avant.prints.iter().filter(|t| t.rarity == rarete).count() as i64
    };

    // ── Branche « ajout, rareté absente de la base » ─────────────────────────
    // La Grand Master Rare n'existe qu'en Overframe : aucune ligne n'en portait
    // avant, il n'y a donc aucune illustration dont hériter.
    assert_eq!(compter_avant("Grand Master Rare"), 0);
    assert_eq!(
        compter("SELECT COUNT(*) FROM set_prints WHERE rarity = 'Grand Master Rare'"),
        18
    );
    assert_eq!(
        compter(
            "SELECT COUNT(*) FROM set_prints
             WHERE rarity = 'Grand Master Rare'
               AND (extended_art <> 1 OR print_image_url IS NOT NULL)"
        ),
        0,
        "les 18 Grand Master Rare doivent être en Overframe et sans URL"
    );

    // ── Branche « ajout, rareté présente dans les deux cadres » ──────────────
    // Les 18 cartes chase gagnent une Ultra Rare Overframe ; leur Ultra Rare
    // normale reste, et l'URL est héritée d'une ligne existante.
    assert_eq!(
        compter("SELECT COUNT(*) FROM set_prints WHERE rarity = 'Ultra Rare'"),
        compter_avant("Ultra Rare") + 18,
        "18 lignes ajoutées, aucune remplacée"
    );
    assert_eq!(
        compter(
            "SELECT COUNT(*) FROM set_prints
             WHERE rarity = 'Ultra Rare' AND extended_art = 1
               AND print_image_url IS NOT NULL"
        ),
        18
    );
    // Chaque numéro qui a gagné une Ultra Rare Overframe garde sa normale.
    assert_eq!(
        compter(
            "SELECT COUNT(*) FROM set_prints n
             WHERE n.rarity = 'Ultra Rare' AND n.extended_art = 0
               AND EXISTS (SELECT 1 FROM set_prints e
                           WHERE e.set_code = n.set_code
                             AND e.rarity = 'Ultra Rare' AND e.extended_art = 1)"
        ),
        18
    );

    // ── Branche « correction de drapeau » ───────────────────────────────────
    // La Prismatic Secret Rare des cartes chase avait été importée en cadre
    // normal. Corriger le drapeau ne crée aucune ligne : le total ne bouge pas.
    assert_eq!(
        compter("SELECT COUNT(*) FROM set_prints WHERE rarity = 'Prismatic Secret Rare'"),
        compter_avant("Prismatic Secret Rare"),
        "une correction de drapeau ne doit jamais ajouter de ligne"
    );
    assert_eq!(
        compter(
            "SELECT COUNT(*) FROM set_prints
             WHERE rarity = 'Prismatic Secret Rare' AND extended_art = 1"
        ),
        18
    );
    // Et aucun doublon : jamais deux lignes de même (numéro, rareté, cadre).
    assert_eq!(
        compter(
            "SELECT COUNT(*) FROM (
                SELECT set_code, rarity, extended_art FROM set_prints
                GROUP BY set_code, rarity, extended_art HAVING COUNT(*) > 1)"
        ),
        0
    );
}

#[test]
fn rejouer_la_passe_ne_change_plus_rien() {
    let f = fixture("bases_LOCR-JP.json");
    let conn = base_avant(&f);
    let classement = classement_reel("wikitext_LOCR-JP_rev5945579.json", "5945579");

    reconcilier(&conn, &f.prefix, &classement).unwrap();
    let apres_un_tour = lire(&conn);

    let bilan = reconcilier(&conn, &f.prefix, &classement).unwrap().unwrap();
    assert_eq!(
        bilan,
        Bilan::default(),
        "la passe doit être idempotente sans même consulter overframe_sync"
    );
    comparer(&lire(&conn), &apres_un_tour, "LOCR-JP, second tour");
}

#[test]
fn ce_que_yugipedia_declare_correspond_a_ce_que_la_base_a_recu() {
    // Contrôle croisé : l'ensemble Overframe lu sur le wiki doit être
    // exactement celui que porte la V1.0.4. S'ils divergeaient, l'épreuve
    // principale passerait quand même — et masquerait le désaccord.
    let f = fixture("bases_LOCR-JP.json");
    let wiki = classement_reel("wikitext_LOCR-JP_rev5945579.json", "5945579");
    let base = classement_depuis(&f);
    assert_eq!(
        wiki.extended, base.extended,
        "les 54 tirages Overframe du wiki et ceux de la base doivent coïncider"
    );
}

#[test]
fn loch_jp_montre_ce_que_la_v1_0_4_n_a_jamais_fait() {
    // Ce set est le cas non traité : Yugipedia y déclare 54 tirages Overframe,
    // et `cardinfo.db` n'en porte aucun. `overframe_sync` de la V1.0.4 ne
    // mentionne d'ailleurs pas LOCH-JP — la passe n'y a jamais abouti.
    //
    // Le portage étant iso-fonctionnel, on ne « corrige » rien : on mesure.
    // Ce test chiffre ce que la passe apporterait, et échouera le jour où
    // quelqu'un décidera d'agir (souhait S11).
    let f = fixture("bases_LOCH-JP.json");
    assert_eq!(f.avant.prints.len(), 279);
    assert_eq!(f.apres.prints.len(), 279, "la V1.0.4 n'a rien ajouté");
    assert!(
        f.apres.prints.iter().all(|t| t.extended_art == 0),
        "et n'a marqué aucun Overframe"
    );

    let classement = classement_reel("wikitext_LOCH-JP_rev5892606.json", "5892606");
    assert_eq!(
        classement.extended.len(),
        54,
        "Yugipedia en déclare pourtant 54"
    );

    let conn = base_avant(&f);
    let bilan = reconcilier(&conn, &f.prefix, &classement).unwrap().unwrap();
    assert_eq!(
        bilan,
        Bilan {
            ajoutes: 34,
            corriges: 17
        },
        "34 et non 36 : un numéro chase manque à cardinfo.db côté LOCH"
    );
    assert_eq!(lire(&conn).len(), 313);
}
