// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Construction de `bdd/cardinfo.db`.
//!
//! Portage de `module/BDD_creation.run_init` et des fonctions d'insertion de
//! `module/db_schema.py`.
//!
//! # Trois invariants, et ce qu'ils coûtent
//!
//! **1. `cards_overrides` n'est JAMAIS supprimée.** Sept tables sont volatiles
//! et recréées à chaque initialisation ; `cards_overrides` est créée en
//! `IF NOT EXISTS` et survit. C'est elle qui protège les corrections manuelles
//! de l'utilisateur. Voir [`TABLES_VOLATILES`].
//!
//! **2. Aucune validation intermédiaire.** Toute l'initialisation est une seule
//! transaction : un échec en cours de route ne doit pas laisser une base à
//! moitié peuplée. Le Python l'obtient par un `BEGIN IMMEDIATE` explicite,
//! parce que son module `sqlite3` valide implicitement avant chaque instruction
//! de définition. Ici, [`crate::connexion::Bases::avec_base`] ouvre la
//! transaction et la garantit par construction.
//!
//! **3. Les index sont créés après les insertions.** Neuf index mis à jour à
//! chaque ligne sur 326 000 tirages, ce serait le poste de coût dominant.
//!
//! # Ce que l'initialisation ne touche pas
//!
//! Trois tables de `cardinfo.db` ne font **pas** partie du schéma d'init :
//! `anomalies`, `overframe_sync` et `card_images_externes`. Elles sont créées
//! à la demande par les fonctionnalités qui les possèdent. C'est observable
//! sur les bases réelles — la V1.0.4, fraîchement initialisée, porte 1 613
//! lignes dans `card_images_externes`.
//!
//! **Ce paragraphe disait « et survivent ». C'était faux.** L'initialisation
//! ne les touche pas, en effet — mais la bascule par fichier temporaire
//! remplace la base *entière*, et ne reprenait que `cards_overrides`. Les
//! trois autres disparaissaient au renommage, en silence. Constaté sur
//! l'installation réelle le 2026-09-05. Elles survivent **depuis** que
//! [`reprendre_tables_hors_init`] existe — ce qui est une propriété du code,
//! pas du fait qu'elles soient hors périmètre.
//!
//! Ces trois tables sont d'ailleurs absentes de l'Annexe A du cahier des
//! charges : les recréer ici, ou les inclure dans la liste des tables
//! volatiles, casserait la détection d'anomalies et la passe Overframe.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Transaction};
use ygo_core::modele::{
    CartesParsees, LigneCarte, LigneImage, LigneLocale, LigneSet, LigneTexte, SetsParses,
    TirageBrut,
};

use crate::error::{DbError, Result};
use crate::schema::{self, DDL_CARDINFO};

/// Tables recréées à chaque initialisation, **dans l'ordre de suppression**.
///
/// L'ordre importe : les clés étrangères vont des tirages vers les sets et les
/// cartes, on supprime donc les dépendants d'abord. Repris tel quel de
/// `db_schema.create_schema`.
pub const TABLES_VOLATILES: [&str; 7] = [
    "set_prints",
    "set_locales",
    "sets",
    "card_texts",
    "card_images",
    "cards",
    "cards_missing_fr",
];

/// Tables de `cardinfo.db` que l'initialisation ne touche pas.
///
/// Documentées ici parce que l'oubli inverse — les inclure dans
/// [`TABLES_VOLATILES`] — est silencieux et destructeur.
///
/// « Ne pas toucher » ne suffit pas à préserver quand la base entière est
/// remplacée : les trois qui ne font pas partie du schéma d'init doivent être
/// **reprises** ([`reprendre_tables_hors_init`]). Ne pas les toucher et les
/// perdre quand même est exactement ce qui est arrivé jusqu'au 2026-09-05.
pub const TABLES_PRESERVEES: [&str; 4] = [
    "cards_overrides",
    "anomalies",
    "overframe_sync",
    "card_images_externes",
];

/// Les huit tables que l'initialisation crée : les volatiles plus
/// `cards_overrides`.
///
/// C'est le périmètre exact sur lequel une base fraîchement initialisée peut
/// être comparée à une base en service.
pub const TABLES_SCHEMA_INIT: [&str; 8] = [
    "set_prints",
    "set_locales",
    "sets",
    "card_texts",
    "card_images",
    "cards",
    "cards_missing_fr",
    "cards_overrides",
];

/// Les trois tables qui n'appartiennent pas au schéma d'initialisation.
///
/// Elles sont créées à la demande par les fonctionnalités qui les possèdent —
/// détection d'anomalies, passe Overframe, artworks Yugipedia. Leur **absence**
/// d'une base fraîchement initialisée est normale, pas une divergence.
pub const TABLES_HORS_INIT: [&str; 3] = ["anomalies", "overframe_sync", "card_images_externes"];

/// Comptages produits par une initialisation, pour le journal et les tests.
///
/// Ce sont exactement les nombres que la V1.0.4 journalise, et donc les
/// assertions de non-régression du portage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Comptages {
    /// Lignes insérées dans `sets`.
    pub sets: usize,
    /// Lignes insérées dans `set_locales`.
    pub locales: usize,
    /// Lignes insérées dans `cards`.
    pub cartes: usize,
    /// Lignes insérées dans `card_texts`.
    pub textes: usize,
    /// Lignes insérées dans `card_images`.
    pub images: usize,
    /// Lignes insérées dans `set_prints`.
    pub tirages: usize,
    /// Lignes insérées dans `cards_missing_fr`.
    pub sans_traduction_fr: usize,
    /// Tirages écartés faute de `set_locale_id` résoluble.
    pub tirages_orphelins: usize,
}

/// Crée le schéma d'initialisation.
///
/// Supprime les sept tables volatiles, les recrée, et crée `cards_overrides`
/// si elle n'existe pas. Les définitions viennent du DDL extrait de la base
/// réelle — elles ne sont pas réécrites ici.
pub fn creer_schema(tx: &Transaction<'_>) -> Result<()> {
    for table in TABLES_VOLATILES {
        tx.execute(&format!("DROP TABLE IF EXISTS {table}"), [])?;
    }

    let attendu = schema::empreinte_attendue(DDL_CARDINFO);
    let mut a_creer: Vec<&str> = TABLES_VOLATILES.to_vec();
    a_creer.push("cards_overrides");

    for table in a_creer {
        let Some(sql) = attendu.get(&("table".to_owned(), table.to_owned())) else {
            return Err(DbError::SchemaNonConforme(1));
        };
        // `cards_overrides` est la seule créée en IF NOT EXISTS : elle survit
        // à l'initialisation, c'est elle qui porte les corrections manuelles.
        if table == "cards_overrides" {
            let existe: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='cards_overrides'",
                [],
                |r| r.get(0),
            )?;
            if existe > 0 {
                continue;
            }
        }
        tx.execute(sql, [])?;
    }
    Ok(())
}

/// Crée les neuf index du schéma d'initialisation.
///
/// À appeler **après** les insertions en masse. `idx_cie_name_set`, qui
/// appartient à `card_images_externes`, n'en fait pas partie.
pub fn creer_index(tx: &Transaction<'_>) -> Result<usize> {
    let attendu = schema::empreinte_attendue(DDL_CARDINFO);
    let mut n = 0;
    for ((genre, nom), sql) in &attendu {
        if genre != "index" || nom == "idx_cie_name_set" {
            continue;
        }
        // `IF NOT EXISTS` comme en Python : l'index peut préexister si la base
        // n'a pas été entièrement recréée.
        let sql = sql.replacen("CREATE INDEX ", "CREATE INDEX IF NOT EXISTS ", 1);
        tx.execute(&sql, [])?;
        n += 1;
    }
    Ok(n)
}

/// Insère les sets.
pub fn inserer_sets(tx: &Transaction<'_>, lignes: &[LigneSet]) -> Result<usize> {
    let mut requete = tx.prepare(
        "INSERT OR IGNORE INTO sets \
         (uuid, name_en, name_fr, name_de, name_it, name_es, name_ja, name_ko) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
    )?;
    for l in lignes {
        requete.execute(params![
            l.uuid, l.name_en, l.name_fr, l.name_de, l.name_it, l.name_es, l.name_ja, l.name_ko
        ])?;
    }
    Ok(lignes.len())
}

/// Insère les déclinaisons locales.
///
/// **L'ordre d'insertion fixe les `set_locales.id`** auto-incrémentés, que
/// `set_prints.set_locale_id` référencera ensuite. C'est la raison pour
/// laquelle le parsing YGOJSON préserve l'ordre d'apparition des clés JSON.
pub fn inserer_locales(tx: &Transaction<'_>, lignes: &[LigneLocale]) -> Result<usize> {
    let mut requete = tx.prepare(
        "INSERT OR IGNORE INTO set_locales \
         (set_uuid, language, prefix, release_date, booster_image_url) \
         VALUES (?1,?2,?3,?4,?5)",
    )?;
    for l in lignes {
        requete.execute(params![
            l.set_uuid,
            l.language,
            l.prefix,
            l.release_date,
            l.booster_image_url
        ])?;
    }
    Ok(lignes.len())
}

/// Insère les cartes.
pub fn inserer_cartes(tx: &Transaction<'_>, lignes: &[LigneCarte]) -> Result<usize> {
    // `def` est un mot réservé de SQL : il doit être cité.
    let mut requete = tx.prepare(
        "INSERT OR IGNORE INTO cards \
         (uuid, ygoprodeck_id, card_type, subcategory, frame_type, \
          atk, \"def\", level, attribute, race, \
          banlist_tcg, banlist_ocg, name_fr_confirmed) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
    )?;
    for l in lignes {
        requete.execute(params![
            l.uuid,
            l.ygoprodeck_id,
            l.card_type,
            l.subcategory,
            l.frame_type,
            l.atk,
            l.def,
            l.level,
            l.attribute,
            l.race,
            l.banlist_tcg,
            l.banlist_ocg,
            l.name_fr_confirmed
        ])?;
    }
    Ok(lignes.len())
}

/// Insère les textes de cartes.
pub fn inserer_textes(tx: &Transaction<'_>, lignes: &[LigneTexte]) -> Result<usize> {
    let mut requete = tx.prepare(
        "INSERT OR IGNORE INTO card_texts (card_uuid, language, name, effect) \
         VALUES (?1,?2,?3,?4)",
    )?;
    for l in lignes {
        requete.execute(params![l.card_uuid, l.language, l.name, l.effect])?;
    }
    Ok(lignes.len())
}

/// Insère les illustrations.
pub fn inserer_images(tx: &Transaction<'_>, lignes: &[LigneImage]) -> Result<usize> {
    let mut requete = tx.prepare(
        "INSERT OR IGNORE INTO card_images \
         (uuid, card_uuid, ygoprodeck_image_id, art_url, card_url) \
         VALUES (?1,?2,?3,?4,?5)",
    )?;
    for l in lignes {
        requete.execute(params![
            l.uuid,
            l.card_uuid,
            l.ygoprodeck_image_id,
            l.art_url,
            l.card_url
        ])?;
    }
    Ok(lignes.len())
}

/// Construit l'index `(set_uuid, langue) → set_locale_id` depuis la base.
pub fn index_locales(tx: &Transaction<'_>) -> Result<HashMap<(String, String), i64>> {
    let mut requete = tx.prepare("SELECT id, set_uuid, language FROM set_locales")?;
    let lignes = requete.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut index = HashMap::new();
    for ligne in lignes {
        let (id, set_uuid, langue) = ligne?;
        index.insert((set_uuid, langue), id);
    }
    Ok(index)
}

/// Insère les tirages en résolvant `set_locale_id`.
///
/// Retourne `(insérés, orphelins)`. Un tirage dont la locale est introuvable
/// est **sauté silencieusement**, comme en Python (`if locale_id is None:
/// continue`) : cela arrive quand un contenu référence une langue que le set ne
/// déclare pas.
pub fn inserer_tirages(
    tx: &Transaction<'_>,
    tirages: &[TirageBrut],
    index: &HashMap<(String, String), i64>,
) -> Result<(usize, usize)> {
    let mut requete = tx.prepare(
        "INSERT INTO set_prints \
         (set_uuid, set_locale_id, card_uuid, card_image_uuid, \
          set_code, rarity, edition, qty, print_image_url) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )?;

    let mut inseres = 0;
    let mut orphelins = 0;
    for t in tirages {
        let Some(locale_id) = index.get(&(t.set_uuid.clone(), t.locale_key.clone())) else {
            orphelins += 1;
            continue;
        };
        requete.execute(params![
            t.set_uuid,
            locale_id,
            t.card_uuid,
            t.card_image_uuid,
            t.set_code,
            t.rarity,
            t.edition,
            t.qty,
            t.print_image_url
        ])?;
        inseres += 1;
    }
    Ok((inseres, orphelins))
}

/// Isole les cartes sans traduction française confirmée.
///
/// Portage de `populate_missing_fr`. L'identifiant de la ligne est le
/// `ygoprodeck_id` de la carte, repris par sous-requête — une carte sans
/// password produit donc une ligne à identifiant nul, et deux cartes sans
/// password entrent en collision de clé primaire : d'où le `INSERT OR IGNORE`.
/// Comportement reproduit tel quel.
pub fn peupler_sans_traduction_fr(
    tx: &Transaction<'_>,
    cartes: &[LigneCarte],
    images: &[LigneImage],
) -> Result<usize> {
    // Première URL d'artwork non vide par carte.
    let mut url_par_carte: HashMap<&str, &str> = HashMap::new();
    for image in images {
        if image.art_url.is_empty() {
            continue;
        }
        url_par_carte
            .entry(image.card_uuid.as_str())
            .or_insert(&image.art_url);
    }

    // Noms anglais, lus depuis la base — ils viennent d'être insérés.
    let mut requete = tx.prepare("SELECT card_uuid, name FROM card_texts WHERE language = 'en'")?;
    let mut noms: HashMap<String, String> = HashMap::new();
    for ligne in requete.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (uuid, nom) = ligne?;
        noms.insert(uuid, nom);
    }
    drop(requete);

    let mut insertion = tx.prepare(
        "INSERT OR IGNORE INTO cards_missing_fr (id, card_uuid, name, card_type, image_url) \
         VALUES ((SELECT ygoprodeck_id FROM cards WHERE uuid = ?1), ?2, ?3, ?4, ?5)",
    )?;

    let mut n = 0;
    for carte in cartes {
        if carte.name_fr_confirmed != 0 {
            continue;
        }
        let Some(nom) = noms.get(carte.uuid.as_str()).filter(|n| !n.is_empty()) else {
            continue;
        };
        insertion.execute(params![
            carte.uuid,
            carte.uuid,
            nom,
            carte.card_type,
            url_par_carte
                .get(carte.uuid.as_str())
                .copied()
                .unwrap_or("")
        ])?;
        n += 1;
    }
    Ok(n)
}

/// Construit `cardinfo.db` **dans une transaction unique**.
///
/// L'appelant fournit une transaction déjà ouverte — typiquement via
/// [`crate::connexion::Bases::avec_base`], qui valide en sortie ou annule tout.
///
/// Ordre des étapes, repris de `run_init` :
///
/// 1. schéma (suppression puis création des tables volatiles) ;
/// 2. sets, locales, cartes, textes, images ;
/// 3. résolution de `set_locale_id`, puis tirages ;
/// 4. **index** ;
/// 5. cartes sans traduction française.
pub fn construire(
    tx: &Transaction<'_>,
    cartes: &CartesParsees,
    sets: &SetsParses,
    tirages: &[TirageBrut],
) -> Result<Comptages> {
    let mut c = Comptages::default();

    creer_schema(tx)?;

    c.sets = inserer_sets(tx, &sets.sets)?;
    c.locales = inserer_locales(tx, &sets.locales)?;
    c.cartes = inserer_cartes(tx, &cartes.cartes)?;
    c.textes = inserer_textes(tx, &cartes.textes)?;
    c.images = inserer_images(tx, &cartes.images)?;

    let index = index_locales(tx)?;
    let (inseres, orphelins) = inserer_tirages(tx, tirages, &index)?;
    c.tirages = inseres;
    c.tirages_orphelins = orphelins;

    creer_index(tx)?;

    c.sans_traduction_fr = peupler_sans_traduction_fr(tx, &cartes.cartes, &cartes.images)?;

    tracing::info!(
        sets = c.sets,
        locales = c.locales,
        cartes = c.cartes,
        textes = c.textes,
        images = c.images,
        tirages = c.tirages,
        orphelins = c.tirages_orphelins,
        sans_fr = c.sans_traduction_fr,
        "cardinfo.db construite"
    );
    Ok(c)
}

/// Reprend les tables hors périmètre d'initialisation d'une base vers l'autre.
///
/// # Le défaut que ceci corrige
///
/// [`TABLES_PRESERVEES`] en annonce quatre. La bascule par fichier temporaire
/// n'en reprenait qu'**une**, `cards_overrides` : les trois autres —
/// [`TABLES_HORS_INIT`] — disparaissaient au renommage, sans un mot.
///
/// Constaté sur l'installation réelle après la mise à jour du 2026-09-05 : les
/// trois tables **absentes**, dont `card_images_externes` et ses 1 613 lignes
/// produites par la V1.0.4. Le module d'à côté affirmait pourtant qu'elles
/// « survivent ». Elles ne survivaient pas ; rien ne le disait.
///
/// # Pourquoi copier le DDL plutôt que le recréer
///
/// Ces tables n'appartiennent pas au schéma d'initialisation — c'est leur
/// définition — et leur DDL vit chez la fonctionnalité qui les possède, en
/// `CREATE TABLE IF NOT EXISTS`. Le recopier ici en ferait une seconde source
/// de vérité, qui divergerait au premier ajout de colonne. On reprend donc le
/// `sql` de `sqlite_master` de la base d'origine : ce qui est repris est
/// exactement ce qui existait, index compris.
///
/// Une table absente de l'ancienne base est simplement sautée : elle n'est
/// créée qu'à l'usage, et une installation qui n'a jamais scanné d'anomalie
/// n'a pas de table `anomalies`.
///
/// # Errors
///
/// Rend une erreur si l'une des deux bases est inutilisable. Une ancienne base
/// **absente** n'en est pas une : c'est le premier démarrage.
pub fn reprendre_tables_hors_init(
    nouvelle: &Path,
    ancienne: &Path,
) -> Result<Vec<(String, usize)>> {
    if !ancienne.is_file() {
        return Ok(Vec::new());
    }
    let conn = crate::connexion::ouvrir(nouvelle)?;
    // ATTACH ne peut pas vivre dans une transaction : la reprise se fait donc
    // table par table, hors transaction. Ce n'est pas un risque — on écrit
    // dans un fichier que personne ne connaît encore.
    conn.execute(
        "ATTACH DATABASE ?1 AS ancienne",
        [&*ancienne.to_string_lossy()],
    )
    .map_err(|e| DbError::sqlite(nouvelle, e))?;

    let mut reprises = Vec::new();
    for table in TABLES_HORS_INIT {
        let Some(ddl) = objet_sql(&conn, "table", table, nouvelle)? else {
            continue;
        };
        conn.execute_batch(&ddl)
            .map_err(|e| DbError::sqlite(nouvelle, e))?;
        let lignes = conn
            .execute(
                &format!("INSERT INTO main.{table} SELECT * FROM ancienne.{table}"),
                [],
            )
            .map_err(|e| DbError::sqlite(nouvelle, e))?;
        for index in index_de(&conn, table, nouvelle)? {
            conn.execute_batch(&index)
                .map_err(|e| DbError::sqlite(nouvelle, e))?;
        }
        reprises.push((table.to_owned(), lignes));
    }

    conn.execute_batch("DETACH DATABASE ancienne")
        .map_err(|e| DbError::sqlite(nouvelle, e))?;
    Ok(reprises)
}

/// Le `CREATE …` d'un objet de la base attachée, s'il existe.
fn objet_sql(
    conn: &rusqlite::Connection,
    genre: &str,
    nom: &str,
    base: &Path,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT sql FROM ancienne.sqlite_master
          WHERE type = ?1 AND name = ?2 AND sql IS NOT NULL",
        params![genre, nom],
        |r| r.get::<_, String>(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        autre => Err(DbError::sqlite(base, autre)),
    })
}

/// Les `CREATE INDEX` d'une table de la base attachée.
///
/// Les index implicites — ceux d'un `UNIQUE` déclaré dans le `CREATE TABLE` —
/// ont un `sql` nul et sont exclus par la requête : ils renaissent avec la
/// table.
fn index_de(conn: &rusqlite::Connection, table: &str, base: &Path) -> Result<Vec<String>> {
    let mut requete = conn
        .prepare(
            "SELECT sql FROM ancienne.sqlite_master
              WHERE type = 'index' AND tbl_name = ?1 AND sql IS NOT NULL",
        )
        .map_err(|e| DbError::sqlite(base, e))?;
    let lignes = requete
        .query_map([table], |r| r.get::<_, String>(0))
        .map_err(|e| DbError::sqlite(base, e))?;
    let mut sql = Vec::new();
    for ligne in lignes {
        sql.push(ligne.map_err(|e| DbError::sqlite(base, e))?);
    }
    Ok(sql)
}

/// Vérifie qu'une base initialisée est utilisable.
///
/// Portage de `verify_database` : les quatre tables essentielles existent, et
/// `cards` comme `set_prints` sont peuplées.
pub fn verifier(chemin: &Path) -> Result<()> {
    let conn = crate::connexion::ouvrir_lecture_seule(chemin)?;

    for table in ["cards", "sets", "set_prints", "card_texts"] {
        let existe: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )?;
        if existe == 0 {
            tracing::error!(table, "table essentielle introuvable");
            return Err(DbError::SchemaNonConforme(1));
        }
    }

    for table in ["cards", "set_prints"] {
        let n: i64 = conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        if n == 0 {
            tracing::error!(table, "table essentielle vide");
            return Err(DbError::SchemaNonConforme(1));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;
    use crate::connexion::Bases;

    fn jeu_minimal() -> (CartesParsees, SetsParses) {
        let cartes = CartesParsees {
            cartes: vec![
                LigneCarte {
                    uuid: "c1".into(),
                    ygoprodeck_id: Some(89631139),
                    card_type: "monster".into(),
                    name_fr_confirmed: 1,
                    ..LigneCarte::default()
                },
                LigneCarte {
                    uuid: "c2".into(),
                    ygoprodeck_id: Some(46986414),
                    card_type: "monster".into(),
                    name_fr_confirmed: 0,
                    ..LigneCarte::default()
                },
            ],
            textes: vec![
                LigneTexte {
                    card_uuid: "c1".into(),
                    language: "en".into(),
                    name: "Blue-Eyes White Dragon".into(),
                    effect: String::new(),
                },
                LigneTexte {
                    card_uuid: "c1".into(),
                    language: "fr".into(),
                    name: "Dragon Blanc aux Yeux Bleus".into(),
                    effect: String::new(),
                },
                LigneTexte {
                    card_uuid: "c2".into(),
                    language: "en".into(),
                    name: "Dark Magician".into(),
                    effect: String::new(),
                },
            ],
            images: vec![
                LigneImage {
                    uuid: "i1".into(),
                    card_uuid: "c1".into(),
                    ygoprodeck_image_id: Some(89631139),
                    art_url: "https://x/89631139.jpg".into(),
                    card_url: String::new(),
                },
                LigneImage {
                    uuid: "i2".into(),
                    card_uuid: "c2".into(),
                    ygoprodeck_image_id: Some(46986414),
                    art_url: "https://x/46986414.jpg".into(),
                    card_url: String::new(),
                },
            ],
            uuids_fr_confirmes: ["c1".to_owned()].into_iter().collect(),
        };

        let sets = SetsParses {
            sets: vec![LigneSet {
                uuid: "s1".into(),
                name_en: "Legend of Blue Eyes".into(),
                name_fr: "Legend of Blue Eyes".into(),
                ..LigneSet::default()
            }],
            locales: vec![
                LigneLocale {
                    set_uuid: "s1".into(),
                    language: "en".into(),
                    prefix: "LOB-EN".into(),
                    release_date: "2002-03-08".into(),
                    booster_image_url: "cover.png".into(),
                },
                LigneLocale {
                    set_uuid: "s1".into(),
                    language: "fr".into(),
                    prefix: "LOB-FR".into(),
                    release_date: String::new(),
                    booster_image_url: String::new(),
                },
            ],
            tirages: vec![],
        };
        (cartes, sets)
    }

    fn tirage(code: &str, langue: &str, carte: &str) -> TirageBrut {
        TirageBrut {
            set_uuid: "s1".into(),
            locale_key: langue.into(),
            card_uuid: carte.into(),
            card_image_uuid: Some(format!("i{}", &carte[1..])),
            set_code: Some(code.into()),
            rarity: "Ultra Rare".into(),
            edition: "1st".into(),
            qty: 1,
            print_image_url: None,
        }
    }

    fn construire_dans(chemin: &Path, tirages: Vec<TirageBrut>) -> Comptages {
        let (cartes, sets) = jeu_minimal();
        let bases = Bases::new();
        bases
            .avec_base(chemin, |tx| construire(tx, &cartes, &sets, &tirages))
            .unwrap()
    }

    #[test]
    fn une_base_neuve_est_conforme_au_ddl_de_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        construire_dans(&base, vec![tirage("LOB-EN001", "en", "c1")]);

        // Les 8 tables du schéma d'init et leurs 9 index doivent correspondre
        // au DDL extrait de la base réelle.
        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        let empreinte = schema::empreinte(&conn).unwrap();
        let attendu = schema::empreinte_attendue(DDL_CARDINFO);

        for (cle, sql) in &empreinte {
            assert_eq!(
                attendu.get(cle),
                Some(sql),
                "définition divergente pour {cle:?}"
            );
        }
        let tables: Vec<&String> = empreinte
            .keys()
            .filter(|(g, _)| g == "table")
            .map(|(_, n)| n)
            .collect();
        assert_eq!(tables.len(), 8, "7 volatiles + cards_overrides");
        let index = empreinte.keys().filter(|(g, _)| g == "index").count();
        assert_eq!(index, 9, "idx_cie_name_set n'appartient pas à l'init");
    }

    #[test]
    fn les_comptages_correspondent_aux_donnees() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let c = construire_dans(
            &base,
            vec![
                tirage("LOB-EN001", "en", "c1"),
                tirage("LOB-FR001", "fr", "c2"),
            ],
        );

        assert_eq!(c.sets, 1);
        assert_eq!(c.locales, 2);
        assert_eq!(c.cartes, 2);
        assert_eq!(c.textes, 3);
        assert_eq!(c.images, 2);
        assert_eq!(c.tirages, 2);
        assert_eq!(c.tirages_orphelins, 0);
        assert_eq!(c.sans_traduction_fr, 1, "seule c2 n'a pas de nom français");

        verifier(&base).unwrap();
    }

    #[test]
    fn un_tirage_dont_la_locale_est_inconnue_est_saute() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let c = construire_dans(
            &base,
            vec![
                tirage("LOB-EN001", "en", "c1"),
                tirage("LOB-DE001", "de", "c1"), // le set ne déclare pas `de`
            ],
        );
        assert_eq!(c.tirages, 1);
        assert_eq!(c.tirages_orphelins, 1);
    }

    #[test]
    fn le_set_locale_id_suit_l_ordre_d_insertion() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        construire_dans(&base, vec![tirage("LOB-EN001", "en", "c1")]);

        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        let mut requete = conn
            .prepare("SELECT id, language FROM set_locales ORDER BY id")
            .unwrap();
        let lignes: Vec<(i64, String)> = requete
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(lignes, vec![(1, "en".to_owned()), (2, "fr".to_owned())]);
    }

    /// L'invariant le plus important du §3.1 : les corrections manuelles de
    /// l'utilisateur survivent à une réinitialisation.
    #[test]
    fn cards_overrides_survit_a_une_reinitialisation() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let bases = Bases::new();

        construire_dans(&base, vec![tirage("LOB-EN001", "en", "c1")]);

        // L'utilisateur enregistre une correction manuelle.
        bases
            .avec_base(&base, |tx| {
                tx.execute(
                    "INSERT INTO cards_overrides \
                     (base_card_id, name, card_sets_set_code, card_sets_set_rarity, reason) \
                     VALUES (?1,?2,?3,?4,?5)",
                    params![
                        89631139,
                        "Blue-Eyes White Dragon",
                        "LOB-EN001",
                        "Ultra Rare",
                        "rareté corrigée à la main"
                    ],
                )?;
                Ok(())
            })
            .unwrap();

        // Réinitialisation complète.
        construire_dans(&base, vec![tirage("LOB-EN001", "en", "c1")]);

        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        let restants: i64 = conn
            .query_row("SELECT count(*) FROM cards_overrides", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            restants, 1,
            "cards_overrides ne doit JAMAIS être supprimée à l'initialisation"
        );
        let raison: String = conn
            .query_row("SELECT reason FROM cards_overrides", [], |r| r.get(0))
            .unwrap();
        assert_eq!(raison, "rareté corrigée à la main");
    }

    /// Les trois tables absentes de l'Annexe A du cahier des charges
    /// appartiennent à d'autres fonctionnalités et doivent survivre elles aussi.
    #[test]
    fn les_listes_de_tables_sont_coherentes() {
        // TABLES_SCHEMA_INIT = les volatiles + cards_overrides, sans doublon.
        assert_eq!(TABLES_SCHEMA_INIT.len(), TABLES_VOLATILES.len() + 1);
        for t in TABLES_VOLATILES {
            assert!(TABLES_SCHEMA_INIT.contains(&t));
        }
        assert!(TABLES_SCHEMA_INIT.contains(&"cards_overrides"));

        // Aucune table ne peut être à la fois créée par l'init et hors init.
        for t in TABLES_HORS_INIT {
            assert!(
                !TABLES_SCHEMA_INIT.contains(&t),
                "`{t}` ne doit pas être créée par l'init"
            );
            assert!(TABLES_PRESERVEES.contains(&t));
        }

        // Ensemble, les deux listes couvrent les 11 tables du DDL réel.
        let tables_ddl = schema::empreinte_attendue(DDL_CARDINFO)
            .keys()
            .filter(|(g, _)| g == "table")
            .count();
        assert_eq!(
            TABLES_SCHEMA_INIT.len() + TABLES_HORS_INIT.len(),
            tables_ddl
        );
        assert_eq!(tables_ddl, 11);
    }

    /// L'init **en place** ne supprime rien — et c'est tout ce que ce test
    /// prouve.
    ///
    /// Il a longtemps passé pour la garantie que les trois tables survivent à
    /// une mise à jour. Elles n'y survivaient pas : le chemin par défaut
    /// remplace le fichier entier, et ce test ne l'emprunte pas. Le vrai
    /// garde-fou est `ygo_app::init::une_reinitialisation_garde_les_tables_…`,
    /// qui passe par `initialiser` avec ses options par défaut, plus
    /// [`les_tables_hors_init_traversent_un_remplacement`] ci-dessous.
    #[test]
    fn les_tables_hors_init_survivent() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let bases = Bases::new();

        construire_dans(&base, vec![tirage("LOB-EN001", "en", "c1")]);

        // Créées et peuplées par leurs modules respectifs.
        let attendu = schema::empreinte_attendue(DDL_CARDINFO);
        bases
            .avec_base(&base, |tx| {
                for table in ["anomalies", "overframe_sync", "card_images_externes"] {
                    let sql = attendu
                        .get(&("table".to_owned(), table.to_owned()))
                        .cloned()
                        .unwrap_or_default();
                    tx.execute(&sql, [])?;
                }
                tx.execute(
                    "INSERT INTO overframe_sync (set_prefix, revid, synced_at) \
                     VALUES ('LOCR-JP', '5945579', '2026-08-25')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        // Réinitialisation.
        construire_dans(&base, vec![tirage("LOB-EN001", "en", "c1")]);

        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        for table in TABLES_PRESERVEES {
            let existe: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(existe, 1, "la table `{table}` a été supprimée par l'init");
        }
        let revid: String = conn
            .query_row("SELECT revid FROM overframe_sync", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            revid, "5945579",
            "sans overframe_sync, la passe Overframe se rejouerait indéfiniment"
        );
    }

    /// L'atomicité du §3.1 : un échec en cours de route ne doit pas laisser une
    /// base à moitié peuplée.
    #[test]
    fn un_echec_en_cours_de_route_n_ecrit_rien() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let bases = Bases::new();
        let (cartes, sets) = jeu_minimal();

        let resultat: Result<()> = bases.avec_base(&base, |tx| {
            construire(tx, &cartes, &sets, &[tirage("LOB-EN001", "en", "c1")])?;
            // Panne simulée après une initialisation complète et réussie.
            Err(DbError::SchemaNonConforme(1))
        });
        assert!(resultat.is_err());

        // Le fichier existe (SQLite l'a créé), mais il est vide de tout schéma.
        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            tables, 0,
            "aucune table ne doit subsister après un rollback"
        );
    }

    #[test]
    fn une_reinitialisation_ne_duplique_rien() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let tirages = vec![tirage("LOB-EN001", "en", "c1")];

        let a = construire_dans(&base, tirages.clone());
        let b = construire_dans(&base, tirages);
        assert_eq!(a, b, "deux passes consécutives donnent le même résultat");

        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM set_prints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn verifier_refuse_une_base_vide() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("vide.db");
        let bases = Bases::new();
        bases.avec_base(&base, creer_schema).unwrap();

        assert!(
            verifier(&base).is_err(),
            "le schéma existe mais `cards` est vide"
        );
    }

    #[test]
    fn la_colonne_def_est_bien_ecrite() {
        // `def` est un mot réservé SQL : sans guillemets, l'insertion échoue.
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("cardinfo.db");
        let bases = Bases::new();
        let (mut cartes, sets) = jeu_minimal();
        cartes.cartes[0].atk = Some(3000);
        cartes.cartes[0].def = Some(2500);
        cartes.cartes[0].frame_type = Some("normal".into());

        bases
            .avec_base(&base, |tx| construire(tx, &cartes, &sets, &[]))
            .unwrap();

        let conn = crate::connexion::ouvrir_lecture_seule(&base).unwrap();
        let (atk, def): (i64, i64) = conn
            .query_row(
                "SELECT atk, \"def\" FROM cards WHERE uuid = 'c1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((atk, def), (3000, 2500));
    }

    /// Le défaut du 2026-09-05, en test : une base remplacée perdait les
    /// tables qu'elle n'a pas créées.
    ///
    /// L'ancienne base porte les trois tables hors init, avec des lignes et un
    /// index. La nouvelle ne les a pas — c'est ce que produit une
    /// construction. Après reprise, les trois doivent être là, à l'identique.
    #[test]
    fn les_tables_hors_init_traversent_un_remplacement() {
        let tmp = tempfile::tempdir().unwrap();
        let ancienne = tmp.path().join("ancienne.db");
        let nouvelle = tmp.path().join("nouvelle.db");

        {
            let conn = crate::connexion::ouvrir(&ancienne).unwrap();
            conn.execute_batch(
                "CREATE TABLE anomalies (
                     id INTEGER PRIMARY KEY,
                     nom TEXT NOT NULL,
                     corrige INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX idx_anomalies_nom ON anomalies(nom);
                 CREATE TABLE overframe_sync (set_prefix TEXT PRIMARY KEY, revid INTEGER);
                 CREATE TABLE card_images_externes (fichier TEXT PRIMARY KEY, rarete TEXT);
                 INSERT INTO anomalies (nom, corrige) VALUES ('Dragon', 1), ('Magicien', 0);
                 INSERT INTO overframe_sync VALUES ('LOCR', 5945579);
                 INSERT INTO card_images_externes VALUES ('RA02-EN006.png', 'ScR');",
            )
            .unwrap();
        }
        {
            // Une « nouvelle base » : une table du schéma d'init, rien d'autre.
            let conn = crate::connexion::ouvrir(&nouvelle).unwrap();
            conn.execute_batch("CREATE TABLE cards (uuid TEXT PRIMARY KEY)")
                .unwrap();
        }

        let reprises = reprendre_tables_hors_init(&nouvelle, &ancienne).unwrap();
        assert_eq!(reprises.len(), 3, "les trois tables : {reprises:?}");

        let conn = crate::connexion::ouvrir_lecture_seule(&nouvelle).unwrap();
        let anomalies: i64 = conn
            .query_row("SELECT count(*) FROM anomalies", [], |r| r.get(0))
            .unwrap();
        assert_eq!(anomalies, 2);

        // Le drapeau `corrige` est la seule chose de cette base qu'aucune
        // reconstruction ne saurait retrouver : il dit ce que l'utilisateur a
        // déjà appliqué.
        let corrige: i64 = conn
            .query_row(
                "SELECT corrige FROM anomalies WHERE nom = 'Dragon'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(corrige, 1);

        let revid: i64 = conn
            .query_row("SELECT revid FROM overframe_sync", [], |r| r.get(0))
            .unwrap();
        assert_eq!(revid, 5_945_579);

        let externes: i64 = conn
            .query_row("SELECT count(*) FROM card_images_externes", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(externes, 1);

        // L'index explicite traverse aussi : sans lui, la table serait là mais
        // la requête qui la lit ne le serait plus.
        let index: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                  WHERE type='index' AND name='idx_anomalies_nom'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index, 1, "l'index de l'ancienne base doit être repris");
    }

    /// Une ancienne base absente est le premier démarrage, pas une erreur.
    #[test]
    fn sans_ancienne_base_il_n_y_a_rien_a_reprendre() {
        let tmp = tempfile::tempdir().unwrap();
        let nouvelle = tmp.path().join("nouvelle.db");
        drop(crate::connexion::ouvrir(&nouvelle).unwrap());
        let reprises =
            reprendre_tables_hors_init(&nouvelle, &tmp.path().join("jamais.db")).unwrap();
        assert!(reprises.is_empty());
    }

    /// Une installation qui n'a jamais scanné d'anomalie n'a pas la table :
    /// elle est sautée, les autres passent.
    #[test]
    fn une_table_absente_de_l_ancienne_base_est_sautee() {
        let tmp = tempfile::tempdir().unwrap();
        let ancienne = tmp.path().join("ancienne.db");
        let nouvelle = tmp.path().join("nouvelle.db");
        {
            let conn = crate::connexion::ouvrir(&ancienne).unwrap();
            conn.execute_batch(
                "CREATE TABLE overframe_sync (set_prefix TEXT PRIMARY KEY, revid INTEGER);
                 INSERT INTO overframe_sync VALUES ('LOCR', 1);",
            )
            .unwrap();
        }
        drop(crate::connexion::ouvrir(&nouvelle).unwrap());

        let reprises = reprendre_tables_hors_init(&nouvelle, &ancienne).unwrap();
        assert_eq!(reprises, vec![("overframe_sync".to_owned(), 1)]);
    }
}
