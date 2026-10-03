// Yu-Gi-Oh! Collection Manager — V2 Rust
// Copyright (C) 2026  Althalusse — GNU GPL v3.0 (cf. lib.rs)

//! Schémas SQLite — création et vérification.
//!
//! Portage de `module/db_schema.py`.
//!
//! # Principe : le DDL n'est pas écrit, il est extrait
//!
//! [`DDL_CARDINFO`] et [`DDL_CLASSEUR`] sont les fichiers `assets/schema/*.sql`,
//! obtenus par `sqlite3 <base> .schema` sur les bases réelles de l'utilisateur.
//! Les créer avec ce texte exact garantit que `sqlite_master` contiendra, au
//! caractère près, ce que contient la base produite par le Python — ce qui rend
//! [`verifier`] utilisable comme test de non-régression strict.
//!
//! # Index créés après les insertions
//!
//! Le Python crée les index **après** le remplissage en masse, pour ne pas
//! payer la mise à jour de dix index par ligne insérée. On conserve cette
//! séparation : [`creer_tables`] puis, en fin de pipeline, [`creer_index`].

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;

use crate::error::{DbError, Result};

/// DDL de `bdd/cardinfo.db` — 11 tables, 10 index.
///
/// Extrait de `Projet Python/V1.0.4/bdd/cardinfo.db`.
pub const DDL_CARDINFO: &str = include_str!("../../../assets/schema/cardinfo.sql");

/// DDL d'une base de classeur — table `cards` (24 colonnes), table `meta`,
/// index `idx_card_id` et `idx_sort`.
///
/// Extrait de `Projet Python/V1.0.3/bdd/classeur_creer/RA05/RA05.db`.
///
/// À noter : l'Annexe A du cahier des charges ne mentionne que `idx_sort` ; la
/// base réelle porte **deux** index.
pub const DDL_CLASSEUR: &str = include_str!("../../../assets/schema/classeur.sql");

/// Une instruction `CREATE …` isolée d'un fichier DDL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Genre {
    /// `CREATE TABLE …`
    Table,
    /// `CREATE INDEX …`
    Index,
}

/// Retire les lignes de commentaire `--` d'un fichier DDL.
///
/// À faire **avant** tout découpage : les commentaires en français contiennent
/// des apostrophes et des points-virgules qui perturberaient le découpage.
pub fn sans_commentaires(ddl: &str) -> String {
    ddl.lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Découpe un fichier DDL en instructions.
///
/// Les commentaires sont d'abord retirés, puis le texte est scindé sur `;`.
/// Le format des fichiers `assets/schema/*.sql` est maîtrisé — une instruction
/// par bloc, aucun `;` dans un littéral texte — ce que vérifie le test
/// `aucun_point_virgule_dans_un_litteral_des_ddl`.
pub fn instructions(ddl: &str) -> Vec<(Genre, String)> {
    sans_commentaires(ddl)
        .split(';')
        .filter_map(|bloc| classer(bloc.trim()))
        .collect()
}

/// Classe un bloc s'il contient une instruction `CREATE TABLE` ou
/// `CREATE INDEX`.
fn classer(sql: &str) -> Option<(Genre, String)> {
    if sql.is_empty() {
        return None;
    }
    let majuscules = sql.to_ascii_uppercase();
    if majuscules.starts_with("CREATE TABLE") {
        Some((Genre::Table, sql.to_owned()))
    } else if majuscules.starts_with("CREATE INDEX")
        || majuscules.starts_with("CREATE UNIQUE INDEX")
    {
        Some((Genre::Index, sql.to_owned()))
    } else {
        None
    }
}

/// Crée les **tables** d'un DDL (sans les index).
pub fn creer_tables(conn: &Connection, ddl: &str) -> Result<usize> {
    executer(conn, ddl, Genre::Table)
}

/// Crée les **index** d'un DDL.
///
/// À appeler en fin de pipeline, après les insertions en masse.
pub fn creer_index(conn: &Connection, ddl: &str) -> Result<usize> {
    executer(conn, ddl, Genre::Index)
}

/// Crée tables puis index — pratique pour une base vide ou un test.
pub fn creer_tout(conn: &Connection, ddl: &str) -> Result<usize> {
    Ok(creer_tables(conn, ddl)? + creer_index(conn, ddl)?)
}

fn executer(conn: &Connection, ddl: &str, genre: Genre) -> Result<usize> {
    let mut n = 0;
    for (g, sql) in instructions(ddl) {
        if g == genre {
            conn.execute(&sql, [])?;
            n += 1;
        }
    }
    Ok(n)
}

/// Empreinte du schéma d'une base : `(genre, nom)` → texte SQL de création.
///
/// Les objets internes de SQLite (`sqlite_*`) et les objets sans SQL (index
/// implicites d'une contrainte `UNIQUE`) sont écartés, comme le fait
/// `sqlite3 .schema`.
pub fn empreinte(conn: &Connection) -> Result<BTreeMap<(String, String), String>> {
    let mut requete = conn.prepare(
        "SELECT type, name, sql FROM sqlite_master \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
         ORDER BY type, name",
    )?;
    let lignes = requete.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut empreinte = BTreeMap::new();
    for ligne in lignes {
        let (genre, nom, sql) = ligne?;
        empreinte.insert((genre, nom), normaliser(&sql));
    }
    Ok(empreinte)
}

/// Empreinte attendue, dérivée d'un fichier DDL.
pub fn empreinte_attendue(ddl: &str) -> BTreeMap<(String, String), String> {
    let mut attendue = BTreeMap::new();
    for (genre, sql) in instructions(ddl) {
        let genre_txt = match genre {
            Genre::Table => "table",
            Genre::Index => "index",
        };
        if let Some(nom) = nom_objet(&sql) {
            attendue.insert((genre_txt.to_owned(), nom), normaliser(&sql));
        }
    }
    attendue
}

/// Extrait le nom de l'objet créé par une instruction `CREATE …`.
fn nom_objet(sql: &str) -> Option<String> {
    let apres_create = sql.split_once(|c: char| c.is_whitespace())?.1;
    let apres_genre = apres_create
        .trim_start()
        .split_once(|c: char| c.is_whitespace())?
        .1;
    let reste = apres_genre.trim_start();
    // `CREATE TABLE IF NOT EXISTS <nom>` — on saute la clause éventuelle.
    let reste = reste
        .strip_prefix("IF NOT EXISTS")
        .map_or(reste, str::trim_start);
    let nom: String = reste
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!nom.is_empty()).then_some(nom)
}

/// Normalise un texte SQL pour la comparaison : espaces compactés, sans
/// point-virgule final.
///
/// Le texte stocké dans `sqlite_master` conserve l'indentation d'origine ; la
/// normalisation évite qu'un simple réalignement du fichier DDL fasse échouer
/// la vérification, sans rien masquer de structurel.
fn normaliser(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(';')
        .trim()
        .to_owned()
}

/// Divergence entre le schéma d'une base et un DDL de référence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Divergence {
    /// Objet attendu, absent de la base.
    Manquant {
        /// `"table"` ou `"index"`.
        genre: String,
        /// Nom de l'objet.
        nom: String,
    },
    /// Objet présent dans la base, absent du DDL de référence.
    EnTrop {
        /// `"table"` ou `"index"`.
        genre: String,
        /// Nom de l'objet.
        nom: String,
    },
    /// Objet présent des deux côtés, mais défini différemment.
    Different {
        /// `"table"` ou `"index"`.
        genre: String,
        /// Nom de l'objet.
        nom: String,
        /// Définition attendue.
        attendu: String,
        /// Définition trouvée.
        trouve: String,
    },
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manquant { genre, nom } => write!(f, "{genre} `{nom}` manquante"),
            Self::EnTrop { genre, nom } => write!(f, "{genre} `{nom}` en trop"),
            Self::Different {
                genre,
                nom,
                attendu,
                trouve,
            } => write!(
                f,
                "{genre} `{nom}` diffère\n    attendu : {attendu}\n    trouvé  : {trouve}"
            ),
        }
    }
}

/// Compare le schéma d'une base au DDL de référence.
///
/// Renvoie la liste (vide si conforme) des divergences.
pub fn verifier(conn: &Connection, ddl: &str) -> Result<Vec<Divergence>> {
    let trouve = empreinte(conn)?;
    let attendu = empreinte_attendue(ddl);
    Ok(comparer(&attendu, &trouve))
}

/// Compare deux empreintes de schéma.
pub fn comparer(
    attendu: &BTreeMap<(String, String), String>,
    trouve: &BTreeMap<(String, String), String>,
) -> Vec<Divergence> {
    let mut divergences = Vec::new();

    for ((genre, nom), sql_attendu) in attendu {
        match trouve.get(&(genre.clone(), nom.clone())) {
            None => divergences.push(Divergence::Manquant {
                genre: genre.clone(),
                nom: nom.clone(),
            }),
            Some(sql_trouve) if sql_trouve != sql_attendu => {
                divergences.push(Divergence::Different {
                    genre: genre.clone(),
                    nom: nom.clone(),
                    attendu: sql_attendu.clone(),
                    trouve: sql_trouve.clone(),
                });
            }
            Some(_) => {}
        }
    }
    for (genre, nom) in trouve.keys() {
        if !attendu.contains_key(&(genre.clone(), nom.clone())) {
            divergences.push(Divergence::EnTrop {
                genre: genre.clone(),
                nom: nom.clone(),
            });
        }
    }
    divergences
}

/// Ouvre une base existante et vérifie son schéma contre un DDL de référence.
pub fn verifier_fichier(chemin: impl AsRef<Path>, ddl: &str) -> Result<Vec<Divergence>> {
    let chemin = chemin.as_ref();
    let conn = Connection::open_with_flags(
        chemin,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| DbError::sqlite(chemin, e))?;
    verifier(&conn, ddl)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn en_memoire() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn le_ddl_cardinfo_contient_11_tables_et_10_index() {
        let inst = instructions(DDL_CARDINFO);
        let tables = inst.iter().filter(|(g, _)| *g == Genre::Table).count();
        let index = inst.iter().filter(|(g, _)| *g == Genre::Index).count();
        assert_eq!(
            tables, 11,
            "cardinfo.db a 11 tables — l'Annexe A du cahier n'en liste que 8"
        );
        assert_eq!(
            index, 10,
            "cardinfo.db a 10 index — l'Annexe A n'en liste que 9"
        );
    }

    #[test]
    fn le_ddl_classeur_contient_2_tables_et_2_index() {
        let inst = instructions(DDL_CLASSEUR);
        assert_eq!(inst.iter().filter(|(g, _)| *g == Genre::Table).count(), 2);
        assert_eq!(
            inst.iter().filter(|(g, _)| *g == Genre::Index).count(),
            2,
            "idx_card_id ET idx_sort — l'Annexe A ne mentionne qu'idx_sort"
        );
    }

    #[test]
    fn les_trois_tables_absentes_du_cahier_sont_bien_la() {
        let noms: Vec<String> = empreinte_attendue(DDL_CARDINFO)
            .keys()
            .filter(|(g, _)| g == "table")
            .map(|(_, n)| n.clone())
            .collect();

        for oubliee in ["anomalies", "overframe_sync", "card_images_externes"] {
            assert!(
                noms.iter().any(|n| n == oubliee),
                "table `{oubliee}` absente du DDL — elle manque à l'Annexe A du cahier des charges"
            );
        }
        assert_eq!(noms.len(), 11);
    }

    /// Le test central : une base créée par le Rust doit avoir, au caractère
    /// près, le schéma de la base produite par la V1.0.4 Python.
    #[test]
    fn cardinfo_creee_par_rust_est_conforme_au_ddl_reel() {
        let conn = en_memoire();
        assert_eq!(creer_tables(&conn, DDL_CARDINFO).unwrap(), 11);
        assert_eq!(creer_index(&conn, DDL_CARDINFO).unwrap(), 10);

        let divergences = verifier(&conn, DDL_CARDINFO).unwrap();
        assert!(
            divergences.is_empty(),
            "divergences :\n{}",
            divergences
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn classeur_cree_par_rust_est_conforme_au_ddl_reel() {
        let conn = en_memoire();
        creer_tout(&conn, DDL_CLASSEUR).unwrap();
        assert!(verifier(&conn, DDL_CLASSEUR).unwrap().is_empty());
    }

    #[test]
    fn le_classeur_a_bien_24_colonnes_dans_l_ordre() {
        // Ordre imposé par le §3.2 du cahier des charges.
        const ATTENDU: [&str; 24] = [
            "card_uuid",
            "card_image_uuid",
            "card_image_id",
            "set_code",
            "rarity",
            "rarity_code",
            "set_name",
            "name",
            "name_fr",
            "card_image_url",
            "card_image_small",
            "sort_order",
            "card_type",
            "atk",
            "def_val",
            "level",
            "attribute",
            "race",
            "possessed",
            "quantite",
            "qualite",
            "edition",
            "extended_art",
            "is_custom",
        ];

        let conn = en_memoire();
        creer_tout(&conn, DDL_CLASSEUR).unwrap();

        let mut requete = conn.prepare("PRAGMA table_info(cards)").unwrap();
        let colonnes: Vec<String> = requete
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .flatten()
            .collect();

        assert_eq!(colonnes, ATTENDU);
    }

    #[test]
    fn creer_index_seul_ne_cree_aucune_table() {
        let conn = en_memoire();
        // Sans les tables, la création d'index doit échouer — c'est la preuve
        // que les deux étapes sont bien distinctes.
        assert!(creer_index(&conn, DDL_CLASSEUR).is_err());
    }

    #[test]
    fn une_table_manquante_est_detectee() {
        let conn = en_memoire();
        creer_tables(&conn, DDL_CARDINFO).unwrap();
        creer_index(&conn, DDL_CARDINFO).unwrap();
        conn.execute("DROP TABLE anomalies", []).unwrap();

        let divergences = verifier(&conn, DDL_CARDINFO).unwrap();
        assert!(divergences.contains(&Divergence::Manquant {
            genre: "table".into(),
            nom: "anomalies".into(),
        }));
    }

    #[test]
    fn une_table_en_trop_est_detectee() {
        let conn = en_memoire();
        creer_tout(&conn, DDL_CLASSEUR).unwrap();
        conn.execute("CREATE TABLE brouillon (x INTEGER)", [])
            .unwrap();

        let divergences = verifier(&conn, DDL_CLASSEUR).unwrap();
        assert!(divergences.contains(&Divergence::EnTrop {
            genre: "table".into(),
            nom: "brouillon".into(),
        }));
    }

    /// Le découpage de [`instructions`] scinde naïvement sur `;`. Ce test
    /// garantit l'hypothèse qui rend ce découpage sûr : aucun `;` n'apparaît à
    /// l'intérieur d'un littéral texte des deux DDL réels. (Il y a bien des
    /// littéraux — `DEFAULT ''`, `datetime('now')` — mais aucun ne contient de
    /// point-virgule.)
    #[test]
    fn aucun_point_virgule_dans_un_litteral_des_ddl() {
        for (nom, ddl) in [("cardinfo", DDL_CARDINFO), ("classeur", DDL_CLASSEUR)] {
            let corps = sans_commentaires(ddl);
            let mut dans_litteral = false;
            for c in corps.chars() {
                match c {
                    '\'' => dans_litteral = !dans_litteral,
                    ';' if dans_litteral => {
                        unreachable!("{nom}.sql : `;` dans un littéral — revoir `instructions()`")
                    }
                    _ => {}
                }
            }
            assert!(!dans_litteral, "{nom}.sql : apostrophe non refermée");
        }
    }
}
